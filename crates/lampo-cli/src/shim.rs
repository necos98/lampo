//! The logic of `lampo-shim.exe` (copied as `php.exe`, `composer.exe`, `mysql.exe`,
//! `mysqldump.exe`, `redis-cli.exe`).
//!
//! Stateless and fast: no async runtime, no IPC, no logging setup. It reads the shim index,
//! resolves the site (`LAMPO_PROJECT` -> cwd -> git worktree -> default), asks the matching
//! [`ShimTool`] for the real invocation and runs it with inherited stdin/stdout/stderr,
//! forwarding the exit code. It works when the app is not running.
//!
//! Own failures print one line `lampo-shim: <reason>` to stderr and exit with 127.

use std::sync::Arc;

use lampo_domain::ports::ShimIndexReader;
use lampo_domain::resolution::{GitInspector, ResolveRequest, ResolverChain};
use lampo_domain::shim::{ShimInvocation, ShimTool, tools};

use crate::console;

/// Exit code when the shim itself cannot run the tool (127, as shells use for "command not found").
pub const UNAVAILABLE: i32 = 127;

/// What the shim was asked to do.
#[derive(Debug, Clone)]
pub struct ShimRequest {
    /// The shim's own executable name or path (`C:\...\bin\php.exe`).
    pub exe: String,
    /// The arguments to hand to the tool, unchanged.
    pub args: Vec<String>,
    pub cwd: String,
    /// `LAMPO_PROJECT`.
    pub explicit_site: Option<String>,
}

/// Resolves the real process to run. The error is the one-line reason for exit code 127.
/// `registered` are the shim tools registered by the composition root (`lampo-shim`'s `main`).
pub fn plan(request: &ShimRequest, registered: &[Arc<dyn ShimTool>], index: &dyn ShimIndexReader, git: Arc<dyn GitInspector>) -> Result<ShimInvocation, String> {
    let tool = tools::for_executable(&request.exe, registered).ok_or_else(|| format!("`{}` is not a Lampo shim name (expected {})", request.exe, tool_names(registered)))?;
    let index = index.read().map_err(|e| format!("cannot read the shim index: {e}"))?;
    let resolve = ResolveRequest { cwd: request.cwd.clone(), explicit_site: request.explicit_site.clone() };
    let resolution = ResolverChain::standard(git)
        .resolve(&resolve, &index)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no Lampo project for this folder and no default PHP: install one in Lampo first (`lampo package install php 8.3`), then run `{}` again", tool.name()))?;
    tool.invocation(&resolution.target, &request.args).map_err(|e| e.to_string())
}

/// `php, composer, mysql, mysqldump or redis-cli`, from the registered tools.
fn tool_names(registered: &[Arc<dyn ShimTool>]) -> String {
    let names: Vec<&str> = registered.iter().map(|t| t.name()).collect();
    match names.split_last() {
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => "no tool (none is registered)".into(),
    }
}

/// Runs the invocation with inherited stdio and returns the tool's exit code. Ctrl+C is left to
/// the child: the shim keeps waiting so the exit code is not lost.
pub fn launch(invocation: &ShimInvocation) -> i32 {
    console::survive_ctrl_c();
    let status = std::process::Command::new(&invocation.program).args(&invocation.args).envs(invocation.env.iter().map(|(k, v)| (k, v))).status();
    match status {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => fail(format!("cannot start {}: {e}", invocation.program.display())),
    }
}

/// Prints the one-line failure and returns exit code 127.
pub fn fail(message: impl std::fmt::Display) -> i32 {
    eprintln!("lampo-shim: {message}");
    UNAVAILABLE
}

/// Name the shim was started as: the executable's own file name (reliable for copies), falling
/// back to `argv[0]`.
pub fn own_name(argv0: &str) -> String {
    std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| argv0.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lampo_domain::PortResult;
    use lampo_domain::php::{ComposerChannel, PhpMinor};
    use lampo_domain::resolution::GitCheckout;
    use lampo_domain::shim::{ShimEntry, ShimIndex, ShimTarget};

    use super::*;

    struct FakeIndex(ShimIndex);
    impl ShimIndexReader for FakeIndex {
        fn read(&self) -> PortResult<ShimIndex> {
            Ok(self.0.clone())
        }
    }

    struct NoGit;
    impl GitInspector for NoGit {
        fn inspect(&self, _cwd: &str) -> PortResult<Option<GitCheckout>> {
            Ok(None)
        }
    }

    fn target(composer: bool) -> ShimTarget {
        let php = PhpMinor::parse("8.3").unwrap();
        ShimTarget {
            php,
            php_dir: PathBuf::from(r"C:\php\8.3"),
            php_exe: PathBuf::from(r"C:\php\8.3\php.exe"),
            phprc_dir: Some(PathBuf::from(r"C:\ini")),
            composer_phar: composer.then(|| PathBuf::from(r"C:\composer\composer.phar")),
            composer_channel: ComposerChannel::for_php(php),
            services: Default::default(),
        }
    }

    fn request(exe: &str, cwd: &str, args: &[&str]) -> ShimRequest {
        ShimRequest { exe: exe.into(), args: args.iter().map(|a| a.to_string()).collect(), cwd: cwd.into(), explicit_site: None }
    }

    fn index_with_project() -> FakeIndex {
        let entry = ShimEntry { site: "shop".parse().unwrap(), root: lampo_domain::project::ProjectRoot::new(r"C:\work\shop").unwrap(), target: target(true) };
        FakeIndex(ShimIndex { version: 1, default: None, projects: vec![entry] })
    }

    #[test]
    fn php_runs_the_projects_binary_with_arguments_unchanged() {
        let args = ["-r", r#"echo "a b";"#, "--", "x y", ""];
        let inv = plan(&request(r"C:\bin\PHP.EXE", r"C:\work\shop\app", &args), &tools::all(), &index_with_project(), Arc::new(NoGit)).unwrap();
        assert_eq!(inv.program, PathBuf::from(r"C:\php\8.3\php.exe"));
        assert_eq!(inv.args, args);
        assert!(inv.env.contains(&("PHPRC".to_string(), r"C:\ini".to_string())));
    }

    #[test]
    fn composer_runs_through_php_and_the_phar() {
        let inv = plan(&request("composer.exe", r"C:\work\shop", &["install"]), &tools::all(), &index_with_project(), Arc::new(NoGit)).unwrap();
        assert_eq!(inv.args, [r"C:\composer\composer.phar", "install"]);
    }

    #[test]
    fn no_project_and_no_default_is_a_clear_one_line_error() {
        let err = plan(&request("php", r"D:\elsewhere", &[]), &tools::all(), &index_with_project(), Arc::new(NoGit)).unwrap_err();
        assert!(!err.contains('\n'));
        assert!(err.contains("no Lampo project"), "{err}");
    }

    #[test]
    fn default_target_serves_folders_outside_projects() {
        let index = FakeIndex(ShimIndex { version: 1, default: Some(target(false)), projects: vec![] });
        let inv = plan(&request("php", r"D:\elsewhere", &["-v"]), &tools::all(), &index, Arc::new(NoGit)).unwrap();
        assert_eq!(inv.program, PathBuf::from(r"C:\php\8.3\php.exe"));
        let err = plan(&request("composer", r"D:\elsewhere", &[]), &tools::all(), &index, Arc::new(NoGit)).unwrap_err();
        assert!(err.contains("Composer"), "{err}");
    }

    #[test]
    fn unknown_shim_name_is_rejected() {
        let err = plan(&request("lampo-shim.exe", r"C:\work\shop", &[]), &tools::all(), &index_with_project(), Arc::new(NoGit)).unwrap_err();
        assert!(err.contains("not a Lampo shim name (expected php, composer, mysql, mysqldump or redis-cli)"), "{err}");
        let only_php: Vec<Arc<dyn ShimTool>> = vec![Arc::new(tools::Php)];
        let err = plan(&request("composer", r"C:workshop", &[]), &only_php, &index_with_project(), Arc::new(NoGit)).unwrap_err();
        assert!(err.ends_with("(expected php)"), "the list follows the registered tools: {err}");
    }

    #[test]
    fn explicit_site_wins_over_the_folder() {
        let mut index = index_with_project();
        let other = ShimEntry {
            site: "blog".parse().unwrap(),
            root: lampo_domain::project::ProjectRoot::new(r"C:\work\blog").unwrap(),
            target: ShimTarget { php_dir: PathBuf::from(r"C:\php\7.4"), php_exe: PathBuf::from(r"C:\php\7.4\php.exe"), ..target(false) },
        };
        index.0.projects.push(other);
        let mut req = request("php", r"C:\work\shop", &[]);
        req.explicit_site = Some("blog".into());
        let inv = plan(&req, &tools::all(), &index, Arc::new(NoGit)).unwrap();
        assert_eq!(inv.program, PathBuf::from(r"C:\php\7.4\php.exe"));
    }
}
