//! `ClaudeCodeCli`: the `claude` program of Claude Code, run hidden through the
//! [`CommandRunner`] (inside the Job Object, `CREATE_NO_WINDOW`).
//!
//! `claude` is looked up on the user's effective PATH (registry, so a fresh install is found
//! without restarting the app), then at the native installer's `~\.local\bin\claude.exe`. A
//! `.exe` runs directly; an npm shim (`claude.cmd`) runs as `cmd.exe /d /c claude <args>` with
//! its folder first on PATH and as the working folder, so that cmd resolves it there and the
//! shim's (possibly spaced) path never needs cmd's quote rules.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::claude_code::parse_version;
use lampo_domain::ports::{ClaudeCodeCli, CommandOutput, CommandRunner, CommandSpec, ExecutableLocator};
use lampo_domain::{PortError, PortResult};

/// `claude --version` must answer within this (a first start of node can be slow).
pub const DETECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Extensions `claude` may have on PATH, in preference order when both sit in one folder.
const EXTENSIONS: &[&str] = &["exe", "cmd", "bat"];

pub struct ClaudeCliRunner {
    runner: Arc<dyn CommandRunner>,
    locator: Arc<dyn ExecutableLocator>,
    /// `%USERPROFILE%`, for the native installer's location.
    home: Option<PathBuf>,
    /// `cmd.exe` (`%ComSpec%`).
    cmd: PathBuf,
}

impl ClaudeCliRunner {
    pub fn new(runner: Arc<dyn CommandRunner>, locator: Arc<dyn ExecutableLocator>, home: Option<PathBuf>) -> Self {
        let cmd = std::env::var_os("ComSpec").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"));
        Self { runner, locator, home, cmd }
    }

    /// The `claude` executable or npm shim, if installed.
    pub fn locate(&self) -> Option<PathBuf> {
        let on_path = match self.locator.find_all("claude") {
            Ok(found) => found,
            Err(e) => {
                tracing::warn!("PATH lookup of claude failed: {e}");
                Vec::new()
            }
        };
        let usable = |p: &PathBuf| extension(p).is_some_and(|ext| EXTENSIONS.contains(&ext.as_str()));
        on_path.into_iter().find(usable).or_else(|| self.home.as_ref().map(|h| h.join(".local").join("bin").join("claude.exe")).filter(|p| p.is_file()))
    }

    fn spec(&self, program: &Path, args: &[String]) -> CommandSpec {
        let dir = program.parent().map(Path::to_path_buf);
        if extension(program).as_deref() == Some("exe") {
            return CommandSpec { program: program.to_path_buf(), args: args.to_vec(), working_dir: dir, ..Default::default() };
        }
        let name = program.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "claude".into());
        let mut cmd_args = vec!["/d".to_string(), "/c".to_string(), name];
        cmd_args.extend(args.iter().cloned());
        CommandSpec { program: self.cmd.clone(), args: cmd_args, working_dir: dir.clone(), path_prepend: dir.into_iter().collect(), ..Default::default() }
    }
}

fn extension(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_lowercase())
}

#[async_trait]
impl ClaudeCodeCli for ClaudeCliRunner {
    async fn detect(&self) -> PortResult<Option<String>> {
        let Some(program) = self.locate() else { return Ok(None) };
        match self.runner.run(self.spec(&program, &["--version".to_string()]), DETECT_TIMEOUT).await {
            Ok(out) if out.success() => Ok(parse_version(&out.stdout).or_else(|| Some("claude".into()))),
            Ok(out) => {
                tracing::warn!("{} --version exited with {:?}: {}", program.display(), out.exit_code, out.stderr.trim());
                Ok(None)
            }
            Err(e) => {
                tracing::warn!("{} --version failed: {e}", program.display());
                Ok(None)
            }
        }
    }

    async fn run(&self, args: &[String], timeout: Duration) -> PortResult<CommandOutput> {
        let program = self.locate().ok_or_else(|| PortError::not_found("program", "claude"))?;
        self.runner.run(self.spec(&program, args), timeout).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct Locator(Vec<PathBuf>);
    impl ExecutableLocator for Locator {
        fn find_all(&self, _: &str) -> PortResult<Vec<PathBuf>> {
            Ok(self.0.clone())
        }
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<CommandSpec>>);
    #[async_trait]
    impl CommandRunner for Recorder {
        async fn run(&self, spec: CommandSpec, _: Duration) -> PortResult<CommandOutput> {
            self.0.lock().unwrap().push(spec);
            Ok(CommandOutput { exit_code: Some(0), stdout: "2.1.0 (Claude Code)\n".into(), stderr: String::new() })
        }
    }

    fn runner(found: &[&str]) -> (Arc<Recorder>, ClaudeCliRunner) {
        let rec = Arc::new(Recorder::default());
        let r = ClaudeCliRunner::new(rec.clone(), Arc::new(Locator(found.iter().map(PathBuf::from).collect())), None);
        (rec, r)
    }

    #[tokio::test]
    async fn npm_shim_runs_through_cmd_with_its_folder_first_on_path() {
        let (rec, r) = runner(&[r"C:\Users\John Doe\AppData\Roaming\npm\claude", r"C:\Users\John Doe\AppData\Roaming\npm\claude.cmd"]);
        assert_eq!(r.detect().await.unwrap().as_deref(), Some("2.1.0 (Claude Code)"));
        r.run(&["plugin".into(), "validate".into(), "C:/a b/x".into()], Duration::from_secs(1)).await.unwrap();
        let specs = rec.0.lock().unwrap().clone();
        let npm = PathBuf::from(r"C:\Users\John Doe\AppData\Roaming\npm");
        assert!(specs[0].program.to_string_lossy().to_lowercase().ends_with("cmd.exe"));
        assert_eq!(specs[0].args, ["/d", "/c", "claude", "--version"]);
        assert_eq!(specs[1].args, ["/d", "/c", "claude", "plugin", "validate", "C:/a b/x"]);
        assert_eq!(specs[1].path_prepend, std::slice::from_ref(&npm));
        assert_eq!(specs[1].working_dir.as_ref(), Some(&npm));
    }

    #[tokio::test]
    async fn native_exe_runs_directly_and_missing_claude_is_none() {
        let (rec, r) = runner(&[r"C:\Users\me\.local\bin\claude.exe"]);
        r.detect().await.unwrap();
        let spec = rec.0.lock().unwrap()[0].clone();
        assert_eq!(spec.program, PathBuf::from(r"C:\Users\me\.local\bin\claude.exe"));
        assert_eq!(spec.args, ["--version"]);
        assert!(spec.path_prepend.is_empty());

        let (rec, r) = runner(&[]);
        assert_eq!(r.detect().await.unwrap(), None);
        assert!(r.run(&["plugin".into()], Duration::from_secs(1)).await.is_err());
        assert!(rec.0.lock().unwrap().is_empty());
    }
}
