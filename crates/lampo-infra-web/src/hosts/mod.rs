//! The Lampo block of the Windows hosts file.
//!
//! * [`merge`] — pure byte-level rewrite of the block (shared with the helper);
//! * [`request`] — the helper's command line and request file, and the block validation;
//! * [`helper`] — the logic of the elevated `lampo-hosts-helper.exe` (and its security rules);
//! * [`elevation`] — how the helper is started (UAC `runas`, or a test launcher);
//! * [`ElevatedHostsFile`] — the `HostsFileReader` / `HostsFileWriter` adapter.

pub mod elevation;
pub mod helper;
pub mod merge;
pub mod request;

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use lampo_domain::hostname::Tld;
use lampo_domain::hosts::HostsBlock;
use lampo_domain::ports::{HostsFileReader, HostsFileWriter};
use lampo_domain::{PortError, PortResult};

use self::elevation::{HelperLauncher, LaunchOutcome};
use self::request::HelperArgs;

const HELPER: &str = "lampo-hosts-helper";
const REQUEST_CONTEXT: &str = "writing the hosts request";

/// Reads the hosts file directly (world-readable). Writes by serialising the desired block to a
/// temp JSON file and launching `lampo-hosts-helper.exe --block <file> --sha256 <digest> --tld
/// <tld>` through the [`HelperLauncher`] (UAC prompt in production), waiting for its exit code.
/// Cancelled prompt → `ElevationDenied`.
///
/// No prompt is shown when the file already contains the desired block, nor for a block the
/// helper would refuse (checked here first). Release builds of the helper always write the
/// system hosts file; `hosts_file` is what this adapter reads (and, in debug builds only, what
/// it asks the helper to write when it is not the system one).
pub struct ElevatedHostsFile {
    hosts_file: PathBuf,
    helper_exe: PathBuf,
    launcher: Arc<dyn HelperLauncher>,
}

impl ElevatedHostsFile {
    /// The system hosts file (`<GetSystemDirectoryW()>\drivers\etc\hosts`, the file the helper
    /// writes), whatever drive Windows is installed on.
    pub fn system(helper_exe: PathBuf, launcher: Arc<dyn HelperLauncher>) -> PortResult<Self> {
        let hosts_file = helper::system_hosts_path().ok_or_else(|| PortError::Unsupported("the system hosts file location is unknown".into()))?;
        Ok(Self::with_launcher(hosts_file, helper_exe, launcher))
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

    /// Debug builds point the helper at a non-system hosts file (tests); release builds never.
    fn hosts_override(&self) -> Option<PathBuf> {
        let is_system = helper::system_hosts_path().is_some_and(|system| system == self.hosts_file);
        (cfg!(debug_assertions) && !is_system).then(|| self.hosts_file.clone())
    }
}

impl HostsFileReader for ElevatedHostsFile {
    fn read_block(&self) -> PortResult<Option<HostsBlock>> {
        Ok(merge::read_block(&self.read_bytes()?))
    }
}

#[async_trait]
impl HostsFileWriter for ElevatedHostsFile {
    async fn write_block(&self, block: &HostsBlock, tld: &Tld) -> PortResult<()> {
        request::validate_request(block, tld).map_err(|e| PortError::invalid_data("hosts block", e))?;
        let original = self.read_bytes()?;
        let desired = merge::merge_block(&original, block).map_err(|e| PortError::invalid_data(self.hosts_file.display().to_string(), e))?;
        if desired == original {
            return Ok(());
        }
        if !self.helper_exe.is_file() {
            return Err(PortError::not_found("hosts helper", self.helper_exe.display().to_string()));
        }

        let (bytes, digest) = request::encode_request(block).map_err(|e| PortError::io(REQUEST_CONTEXT, e))?;
        // Removed when dropped, right after the helper exits.
        let mut request_file = tempfile::Builder::new().prefix("lampo-hosts-").suffix(".json").tempfile().map_err(|e| PortError::io(REQUEST_CONTEXT, e))?;
        request_file.write_all(&bytes).and_then(|()| request_file.flush()).map_err(|e| PortError::io(REQUEST_CONTEXT, e))?;
        let args = HelperArgs { block_file: request_file.path().to_path_buf(), digest, tld: tld.clone(), hosts_override: self.hosts_override() }.to_args();
        let (launcher, helper_exe) = (self.launcher.clone(), self.helper_exe.clone());
        let outcome = tokio::task::spawn_blocking(move || launcher.run(&helper_exe, &args)).await.map_err(|e| PortError::process(HELPER, e))??;
        drop(request_file);

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
    /// The request file is missing, too large, does not match its digest, is not a block, or
    /// contains non-loopback addresses or names outside the local TLDs.
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
                let parsed = HelperArgs::parse(args).unwrap();
                let block = request::load_request(&parsed).unwrap();
                assert_eq!(helper::apply(target, &block), helper_exit::OK);
            }
            Ok(self.outcome)
        }
    }

    fn block_of(host: &str) -> HostsBlock {
        HostsBlock::new([HostsEntry { ip: IpAddr::V4(Ipv4Addr::LOCALHOST), hostname: Hostname::new(host).unwrap() }])
    }

    fn block() -> HostsBlock {
        block_of("shop.test")
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
        file.write_block(&block(), &Tld::default()).await.unwrap();
        assert_eq!(file.read_block().unwrap(), Some(block()));
        let calls = launcher.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let args = HelperArgs::parse(&calls[0]).unwrap();
        assert_eq!(args.tld, Tld::default());
        assert_eq!(args.hosts_override, Some(file.hosts_file.clone()), "debug builds pass the non-system hosts path");
        assert!(!args.block_file.exists(), "request file removed");
    }

    #[tokio::test]
    async fn the_configured_tld_is_passed_to_the_helper() {
        let (_dir, launcher, file) = setup(LaunchOutcome::Exited(0), true);
        let lan = Tld::new("lan").unwrap();
        file.write_block(&block_of("shop.lan"), &lan).await.unwrap();
        assert_eq!(HelperArgs::parse(&launcher.calls.lock().unwrap()[0]).unwrap().tld, lan);
    }

    #[tokio::test]
    async fn no_prompt_when_the_block_is_already_there() {
        let (_dir, launcher, file) = setup(LaunchOutcome::Exited(0), true);
        file.write_block(&block(), &Tld::default()).await.unwrap();
        file.write_block(&block(), &Tld::default()).await.unwrap();
        assert_eq!(launcher.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn refused_prompt_is_elevation_denied() {
        let (_dir, _launcher, file) = setup(LaunchOutcome::Denied, false);
        assert_eq!(file.write_block(&block(), &Tld::default()).await, Err(PortError::ElevationDenied));
    }

    #[tokio::test]
    async fn helper_failures_and_silent_reverts_are_errors() {
        let (_dir, _launcher, file) = setup(LaunchOutcome::Exited(helper_exit::WRITE_FAILED), false);
        assert!(matches!(file.write_block(&block(), &Tld::default()).await, Err(PortError::Io { .. })));
        let (_dir, _launcher, file) = setup(LaunchOutcome::Exited(0), false);
        assert!(matches!(file.write_block(&block(), &Tld::default()).await, Err(PortError::Process { .. })));
    }

    #[tokio::test]
    async fn refused_blocks_never_reach_the_helper() {
        let (_dir, launcher, file) = setup(LaunchOutcome::Exited(0), true);
        let evil = HostsBlock::new([HostsEntry { ip: "203.0.113.9".parse().unwrap(), hostname: Hostname::new("bank.test").unwrap() }]);
        assert!(matches!(file.write_block(&evil, &Tld::default()).await, Err(PortError::InvalidData { .. })));
        let public = block_of("windowsupdate.com");
        assert!(matches!(file.write_block(&public, &Tld::default()).await, Err(PortError::InvalidData { .. })));
        assert!(launcher.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn the_system_hosts_file_needs_no_override() {
        let file =
            ElevatedHostsFile::system(PathBuf::from("lampo-hosts-helper.exe"), Arc::new(FakeLauncher { outcome: LaunchOutcome::Denied, calls: Mutex::new(Vec::new()), apply_to: None }));
        if cfg!(windows) {
            let file = file.unwrap();
            assert_eq!(Some(file.hosts_file.clone()), helper::system_hosts_path());
            assert_eq!(file.hosts_override(), None);
        } else {
            assert!(matches!(file, Err(PortError::Unsupported(_))));
        }
    }
}
