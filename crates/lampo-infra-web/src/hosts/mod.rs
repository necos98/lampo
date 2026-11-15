//! The Lampo block of the Windows hosts file.
//!
//! * [`merge`] — pure byte-level rewrite of the block (shared with the helper);
//! * [`helper`] — the logic of the elevated `lampo-hosts-helper.exe` (and its security rules);
//! * [`elevation`] — how the helper is started (UAC `runas`, or a test launcher);
//! * [`ElevatedHostsFile`] — the `HostsFileReader` / `HostsFileWriter` adapter.

pub mod elevation;
pub mod helper;
pub mod merge;

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use lampo_domain::hosts::HostsBlock;
use lampo_domain::ports::{HostsFileReader, HostsFileWriter};
use lampo_domain::{PortError, PortResult};

use self::elevation::{HelperLauncher, LaunchOutcome, RunAsLauncher};

/// Default hosts file location.
pub const HOSTS_FILE: &str = r"C:\Windows\System32\drivers\etc\hosts";

const HELPER: &str = "lampo-hosts-helper";

/// Reads the hosts file directly (world-readable). Writes by serialising the desired block to a
/// temp JSON file and launching `lampo-hosts-helper.exe --block <file>` with `ShellExecuteExW`
/// verb `runas` (UAC prompt), waiting for its exit code. Cancelled prompt → `ElevationDenied`.
///
/// No prompt is shown when the file already contains the desired block. Release builds of the
/// helper always write the system hosts file; `hosts_file` is what this adapter reads (and, in
/// debug builds only, what it asks the helper to write).
pub struct ElevatedHostsFile {
    hosts_file: PathBuf,
    helper_exe: PathBuf,
    launcher: Arc<dyn HelperLauncher>,
}

impl ElevatedHostsFile {
    pub fn new(hosts_file: PathBuf, helper_exe: PathBuf) -> Self {
        Self::with_launcher(hosts_file, helper_exe, Arc::new(RunAsLauncher::default()))
    }

    pub fn with_launcher(hosts_file: PathBuf, helper_exe: PathBuf, launcher: Arc<dyn HelperLauncher>) -> Self {
        Self { hosts_file, helper_exe, launcher }
    }

    fn read_bytes(&self) -> PortResult<Vec<u8>> {
        match std::fs::read(&self.hosts_file) {
            Ok(b) => Ok(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(PortError::io(format!("reading {}", self.hosts_file.display()), e)),
        }
    }

    fn helper_args(&self, request: &std::path::Path) -> Vec<String> {
        let mut args = vec!["--block".to_string(), request.display().to_string()];
        if cfg!(debug_assertions) && !self.hosts_file.as_os_str().eq_ignore_ascii_case(HOSTS_FILE) {
            args.extend(["--hosts".to_string(), self.hosts_file.display().to_string()]);
        }
        args
    }
}

impl HostsFileReader for ElevatedHostsFile {
    fn read_block(&self) -> PortResult<Option<HostsBlock>> {
        Ok(merge::read_block(&self.read_bytes()?))
    }
}

#[async_trait]
impl HostsFileWriter for ElevatedHostsFile {
    async fn write_block(&self, block: &HostsBlock) -> PortResult<()> {
        if !helper::validate_request(block) {
            return Err(PortError::invalid_data("hosts block", "only loopback addresses (at most 4096 entries) can be written"));
        }
        let original = self.read_bytes()?;
        let desired = merge::merge_block(&original, block).map_err(|e| PortError::invalid_data(self.hosts_file.display().to_string(), e))?;
        if desired == original {
            return Ok(());
        }
        if !self.helper_exe.is_file() {
            return Err(PortError::not_found("hosts helper", self.helper_exe.display().to_string()));
        }

        let mut request = tempfile::Builder::new().prefix("lampo-hosts-").suffix(".json").tempfile().map_err(|e| PortError::io("writing the hosts request", e))?;
        serde_json::to_writer(&mut request, block).map_err(|e| PortError::io("writing the hosts request", e))?;
        request.flush().map_err(|e| PortError::io("writing the hosts request", e))?;
        let args = self.helper_args(request.path());
        let (launcher, helper_exe) = (self.launcher.clone(), self.helper_exe.clone());
        let outcome = tokio::task::spawn_blocking(move || launcher.run(&helper_exe, &args)).await.map_err(|e| PortError::process(HELPER, e))??;
        drop(request);

        match outcome {
            LaunchOutcome::Denied => Err(PortError::ElevationDenied),
            LaunchOutcome::Exited(helper_exit::OK) => {
                // Antivirus products may silently revert hosts-file changes.
                let now = self.read_block()?;
                let expected = (!block.is_empty()).then(|| block.clone());
                if now == expected { Ok(()) } else { Err(PortError::process(HELPER, "the hosts file was not updated (blocked by antivirus or another program?)")) }
            }
            LaunchOutcome::Exited(code) => Err(helper_exit::to_error(code, &self.hosts_file)),
        }
    }
}

/// Exit codes of `lampo-hosts-helper` (contract between the helper and [`ElevatedHostsFile`]).
pub mod helper_exit {
    use std::path::Path;

    use lampo_domain::PortError;

    pub const OK: i32 = 0;
    pub const BAD_ARGUMENTS: i32 = 2;
    pub const READ_FAILED: i32 = 3;
    pub const WRITE_FAILED: i32 = 4;
    /// The request file is missing, too large, not a block, or contains non-loopback addresses.
    pub const INVALID_REQUEST: i32 = 5;
    /// The hosts file is UTF-16 or has an unterminated Lampo block: left untouched.
    pub const UNSUPPORTED_HOSTS_FILE: i32 = 6;

    pub fn to_error(code: i32, hosts_file: &Path) -> PortError {
        let file = hosts_file.display().to_string();
        match code {
            READ_FAILED => PortError::io(format!("reading {file}"), "the helper could not read the hosts file"),
            WRITE_FAILED => PortError::io(format!("writing {file}"), "the helper could not replace the hosts file (read-only or locked?)"),
            INVALID_REQUEST => PortError::invalid_data("hosts block", "the helper rejected the request"),
            UNSUPPORTED_HOSTS_FILE => PortError::invalid_data(file, "UTF-16 encoding or an incomplete Lampo block; fix the file manually"),
            BAD_ARGUMENTS => PortError::process(super::HELPER, "the helper rejected its arguments (version mismatch?)"),
            other => PortError::process(super::HELPER, format!("exited with code {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Mutex;

    use lampo_domain::hostname::Hostname;
    use lampo_domain::hosts::HostsEntry;

    use super::*;

    /// Records calls and simulates the helper in-process (never elevates).
    struct FakeLauncher {
        outcome: LaunchOutcome,
        calls: Mutex<Vec<Vec<String>>>,
        apply_to: Option<PathBuf>,
    }

    impl HelperLauncher for FakeLauncher {
        fn run(&self, _helper: &std::path::Path, args: &[String]) -> PortResult<LaunchOutcome> {
            self.calls.lock().unwrap().push(args.to_vec());
            if let (Some(target), LaunchOutcome::Exited(0)) = (&self.apply_to, self.outcome) {
                let parsed = helper::parse_args(args).unwrap();
                let block = helper::load_request(&parsed.block_file).unwrap();
                assert_eq!(helper::apply(target, &block), helper_exit::OK);
            }
            Ok(self.outcome)
        }
    }

    fn block() -> HostsBlock {
        HostsBlock::new([HostsEntry { ip: IpAddr::V4(Ipv4Addr::LOCALHOST), hostname: Hostname::new("shop.test").unwrap() }])
    }

    fn setup(outcome: LaunchOutcome, simulate: bool) -> (tempfile::TempDir, Arc<FakeLauncher>, ElevatedHostsFile) {
        let dir = tempfile::tempdir().unwrap();
        let hosts = dir.path().join("hosts");
        std::fs::write(&hosts, b"127.0.0.1 localhost\r\n").unwrap();
        let helper = dir.path().join("lampo-hosts-helper.exe");
        std::fs::write(&helper, b"").unwrap();
        let launcher = Arc::new(FakeLauncher { outcome, calls: Mutex::new(Vec::new()), apply_to: simulate.then(|| hosts.clone()) });
        let file = ElevatedHostsFile::with_launcher(hosts, helper, launcher.clone());
        (dir, launcher, file)
    }

    #[tokio::test]
    async fn writes_through_the_helper_then_verifies() {
        let (_dir, launcher, file) = setup(LaunchOutcome::Exited(0), true);
        assert_eq!(file.read_block().unwrap(), None);
        file.write_block(&block()).await.unwrap();
        assert_eq!(file.read_block().unwrap(), Some(block()));
        let calls = launcher.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0], "--block");
        assert_eq!(calls[0][2], "--hosts", "debug builds pass the non-default hosts path");
        assert!(!PathBuf::from(&calls[0][1]).exists(), "request file removed");
    }

    #[tokio::test]
    async fn no_prompt_when_the_block_is_already_there() {
        let (_dir, launcher, file) = setup(LaunchOutcome::Exited(0), true);
        file.write_block(&block()).await.unwrap();
        file.write_block(&block()).await.unwrap();
        assert_eq!(launcher.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn refused_prompt_is_elevation_denied() {
        let (_dir, _launcher, file) = setup(LaunchOutcome::Denied, false);
        assert_eq!(file.write_block(&block()).await, Err(PortError::ElevationDenied));
    }

    #[tokio::test]
    async fn helper_failures_and_silent_reverts_are_errors() {
        let (_dir, _launcher, file) = setup(LaunchOutcome::Exited(helper_exit::WRITE_FAILED), false);
        assert!(matches!(file.write_block(&block()).await, Err(PortError::Io { .. })));
        let (_dir, _launcher, file) = setup(LaunchOutcome::Exited(0), false);
        assert!(matches!(file.write_block(&block()).await, Err(PortError::Process { .. })));
    }

    #[tokio::test]
    async fn non_loopback_blocks_never_reach_the_helper() {
        let (_dir, launcher, file) = setup(LaunchOutcome::Exited(0), true);
        let evil = HostsBlock::new([HostsEntry { ip: "203.0.113.9".parse().unwrap(), hostname: Hostname::new("bank.test").unwrap() }]);
        assert!(matches!(file.write_block(&evil).await, Err(PortError::InvalidData { .. })));
        assert!(launcher.calls.lock().unwrap().is_empty());
    }
}
