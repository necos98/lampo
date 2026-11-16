//! The logic of `lampo-shim.exe` (copied as `php.exe`, `composer.exe`, `mysql.exe`,
//! `mysqldump.exe`, `redis-cli.exe`).
//!
//! Stateless and fast: no async runtime, no IPC, no logging setup. It reads the shim index,
//! resolves the site (`LAMPO_PROJECT` while it holds -> cwd -> git worktree -> default), asks the
//! matching [`ShimTool`] for the real invocation (a MySQL client gets the stored password of its
//! account through an option file, [`with_stored_credentials`]) and runs it with inherited stdin/stdout/stderr,
//! tied to the shim's lifetime ([`crate::job`]), forwarding the exit code. It works when the app
//! is not running.
//!
//! Own failures print one line `lampo-shim: <reason>` to stderr and exit with 127.

use std::sync::Arc;

use lampo_domain::ports::{MysqlProjectPasswordReader, MysqlRootPasswordReader, ShimIndexReader};
use lampo_domain::resolution::{GitInspector, PathCanonicalizer, ResolveRequest, ResolverChain};
use lampo_domain::shim::{FolderProbe, ShimInvocation, ShimTool, StoredCredentials, tools};
use lampo_infra_services::ClientOptionFile;

use crate::{console, job};

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
    pub inherited_site: Option<String>,
    /// `LAMPO_PROJECT_ROOT`.
    pub inherited_root: Option<String>,
}

/// The file-system facts the shim reads besides the index: for resolution, and the folders a
/// tool's defaults depend on (the site's database in MySQL's data dir).
pub struct ShimFacts {
    pub git: Arc<dyn GitInspector>,
    pub canonicalizer: Arc<dyn PathCanonicalizer>,
    pub folders: Arc<dyn FolderProbe>,
}

/// What to run, and the notes to print first (`LAMPO_PROJECT` was ignored for this folder, the
/// site's database does not exist yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimPlan {
    pub invocation: ShimInvocation,
    pub notices: Vec<String>,
}

/// Resolves the real process to run. The error is the one-line reason for exit code 127.
/// `registered` are the shim tools registered by the composition root (`lampo-shim`'s `main`).
pub fn plan(request: &ShimRequest, registered: &[Arc<dyn ShimTool>], index: &dyn ShimIndexReader, facts: ShimFacts) -> Result<ShimPlan, String> {
    let tool = tools::for_executable(&request.exe, registered).ok_or_else(|| format!("`{}` is not a Lampo shim name (expected {})", request.exe, tool_names(registered)))?;
    let index = index.read().map_err(|e| format!("cannot read the shim index: {e}"))?;
    let resolve = ResolveRequest { cwd: request.cwd.clone(), explicit_site: None, inherited_site: request.inherited_site.clone(), inherited_root: request.inherited_root.clone() };
    let resolution = ResolverChain::standard(facts.git, facts.canonicalizer)
        .resolve(&resolve, &index)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no Lampo project for this folder and no default PHP: install one in Lampo first (`lampo package install php 8.3`), then run `{}` again", tool.name()))?;
    let mut invocation = tool.invocation(&resolution.target, &request.args, facts.folders.as_ref()).map_err(|e| e.to_string())?;
    let notices = resolution.ignored_inherited_notice().into_iter().chain(invocation.notice.take()).collect();
    Ok(ShimPlan { invocation, notices })
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

/// An invocation ready to launch, and the option file it reads: keep it until the tool exits.
#[derive(Debug)]
pub struct PreparedInvocation {
    pub invocation: ShimInvocation,
    /// Held for its lifetime only: dropping it deletes the file.
    _option_file: Option<ClientOptionFile>,
}

/// The stored passwords a shim may hand to a MySQL client.
pub struct StoredPasswords<'a> {
    pub root: &'a dyn MysqlRootPasswordReader,
    pub projects: &'a dyn MysqlProjectPasswordReader,
}

/// Gives a MySQL client the stored password its invocation asks for
/// ([`ShimInvocation::stored_credentials`]: the instance's root or the project's dedicated
/// account), when one is stored: an option file only the user can open, read first
/// (`--defaults-extra-file=`, so the user's own option files keep applying), never the command
/// line. The error is the one-line reason for exit code 127.
pub fn with_stored_credentials(mut invocation: ShimInvocation, passwords: &StoredPasswords<'_>) -> Result<PreparedInvocation, String> {
    let option_file = match &invocation.stored_credentials {
        Some(StoredCredentials::Root(instance)) => {
            ClientOptionFile::for_instance(passwords.root, instance).map_err(|e| format!("cannot use the root password Lampo stored for {instance}: {e}"))?
        }
        Some(StoredCredentials::Project(project)) => {
            let unusable = |e: lampo_domain::PortError| format!("cannot use the MySQL password Lampo stored for project {project}: {e}");
            passwords.projects.project_password(project).map_err(unusable)?.map(|p| ClientOptionFile::for_password(&p)).transpose().map_err(unusable)?
        }
        None => None,
    };
    if let Some(file) = &option_file {
        invocation.args.insert(0, file.defaults_extra_file_arg());
    }
    Ok(PreparedInvocation { invocation, _option_file: option_file })
}

/// Runs the invocation with inherited stdio, tied to the shim ([`job::run_tied`]), and returns
/// the tool's exit code. Ctrl+C is left to the child: the shim keeps waiting so the exit code is
/// not lost.
pub fn launch(invocation: &ShimInvocation) -> i32 {
    console::survive_ctrl_c();
    let mut command = std::process::Command::new(&invocation.program);
    command.args(&invocation.args).envs(invocation.env.iter().map(|(k, v)| (k, v)));
    match job::run_tied(&mut command) {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => fail(format!("cannot start {}: {e}", invocation.program.display())),
    }
}

/// Prints the one-line failure and returns exit code 127.
pub fn fail(message: impl std::fmt::Display) -> i32 {
    note(message);
    UNAVAILABLE
}

/// One line `lampo-shim: <message>` on stderr.
pub fn note(message: impl std::fmt::Display) {
    eprintln!("lampo-shim: {message}");
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
    use lampo_domain::mysql::credentials::MysqlPassword;
    use lampo_domain::php::{ComposerChannel, PhpMinor};
    use lampo_domain::ports::NoMysqlRootPasswords;
    use lampo_domain::project::{ProjectId, ProjectRoot};
    use lampo_domain::resolution::GitCheckout;
    use lampo_domain::service::{ServiceId, ServiceKind};
    use lampo_domain::shim::{FolderState, ShimEntry, ShimIndex, ShimService, ShimTarget};
    use lampo_domain::tcp_port::Port;

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

    struct SameName;
    impl PathCanonicalizer for SameName {
        fn canonicalize(&self, folder: &str) -> Option<String> {
            Some(folder.to_string())
        }
    }

    struct EveryFolder;
    impl FolderProbe for EveryFolder {
        fn state(&self, _path: &std::path::Path) -> FolderState {
            FolderState::Present
        }
    }

    fn facts() -> ShimFacts {
        ShimFacts { git: Arc::new(NoGit), canonicalizer: Arc::new(SameName), folders: Arc::new(EveryFolder) }
    }

    fn invocation(request: &ShimRequest, registered: &[Arc<dyn ShimTool>], index: &FakeIndex) -> Result<ShimInvocation, String> {
        plan(request, registered, index, facts()).map(|p| p.invocation)
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
            env_vars: Default::default(),
        }
    }

    fn request(exe: &str, cwd: &str, args: &[&str]) -> ShimRequest {
        ShimRequest { exe: exe.into(), args: args.iter().map(|a| a.to_string()).collect(), cwd: cwd.into(), inherited_site: None, inherited_root: None }
    }

    fn index_with_project() -> FakeIndex {
        let entry = ShimEntry { site: "shop".parse().unwrap(), root: ProjectRoot::new(r"C:\work\shop").unwrap(), target: target(true) };
        FakeIndex(ShimIndex { version: 1, default: None, projects: vec![entry] })
    }

    /// `shop` (PHP 8.3) and `blog` (PHP 7.4).
    fn index_with_two_projects() -> FakeIndex {
        let mut index = index_with_project();
        let blog = ShimEntry {
            site: "blog".parse().unwrap(),
            root: ProjectRoot::new(r"C:\work\blog").unwrap(),
            target: ShimTarget { php_dir: PathBuf::from(r"C:\php\7.4"), php_exe: PathBuf::from(r"C:\php\7.4\php.exe"), ..target(false) },
        };
        index.0.projects.push(blog);
        index
    }

    #[test]
    fn php_runs_the_projects_binary_with_arguments_unchanged() {
        let args = ["-r", r#"echo "a b";"#, "--", "x y", ""];
        let inv = invocation(&request(r"C:\bin\PHP.EXE", r"C:\work\shop\app", &args), &tools::all(), &index_with_project()).unwrap();
        assert_eq!(inv.program, PathBuf::from(r"C:\php\8.3\php.exe"));
        assert_eq!(inv.args, args);
        assert!(inv.env.contains(&("PHPRC".to_string(), r"C:\ini".to_string())));
    }

    #[test]
    fn composer_runs_through_php_and_the_phar() {
        let inv = invocation(&request("composer.exe", r"C:\work\shop", &["install"]), &tools::all(), &index_with_project()).unwrap();
        assert_eq!(inv.args, [r"C:\composer\composer.phar", "install"]);
    }

    #[test]
    fn no_project_and_no_default_is_a_clear_one_line_error() {
        let err = invocation(&request("php", r"D:\elsewhere", &[]), &tools::all(), &index_with_project()).unwrap_err();
        assert!(!err.contains('\n'));
        assert!(err.contains("no Lampo project"), "{err}");
    }

    #[test]
    fn default_target_serves_folders_outside_projects() {
        let index = FakeIndex(ShimIndex { version: 1, default: Some(target(false)), projects: vec![] });
        let inv = invocation(&request("php", r"D:\elsewhere", &["-v"]), &tools::all(), &index).unwrap();
        assert_eq!(inv.program, PathBuf::from(r"C:\php\8.3\php.exe"));
        let err = invocation(&request("composer", r"D:\elsewhere", &[]), &tools::all(), &index).unwrap_err();
        assert!(err.contains("Composer"), "{err}");
    }

    #[test]
    fn unknown_shim_name_is_rejected() {
        let err = invocation(&request("lampo-shim.exe", r"C:\work\shop", &[]), &tools::all(), &index_with_project()).unwrap_err();
        assert!(err.contains("not a Lampo shim name (expected php, composer, mysql, mysqldump or redis-cli)"), "{err}");
        let only_php: Vec<Arc<dyn ShimTool>> = vec![Arc::new(tools::Php)];
        let err = invocation(&request("composer", r"C:workshop", &[]), &only_php, &index_with_project()).unwrap_err();
        assert!(err.ends_with("(expected php)"), "the list follows the registered tools: {err}");
    }

    #[test]
    fn verbatim_cwd_finds_its_project() {
        let inv = invocation(&request("php", r"\\?\C:\work\shop\app", &[]), &tools::all(), &index_with_project()).unwrap();
        assert_eq!(inv.program, PathBuf::from(r"C:\php\8.3\php.exe"));
    }

    /// In the terminal of `blog` (`LAMPO_PROJECT=blog`), `cd ..\shop; php ...` runs shop's PHP.
    #[test]
    fn inherited_project_gives_way_to_the_folder_of_another_project() {
        let mut req = request("php", r"C:\work\shop", &[]);
        req.inherited_site = Some("blog".into());
        req.inherited_root = Some(r"C:\work\blog".into());
        let planned = plan(&req, &tools::all(), &index_with_two_projects(), facts()).unwrap();
        assert_eq!(planned.invocation.program, PathBuf::from(r"C:\php\8.3\php.exe"));
        assert_eq!(planned.notices, ["LAMPO_PROJECT=blog ignored: this folder belongs to shop (set LAMPO_QUIET=1 to hide this note)"]);
    }

    #[test]
    fn inherited_project_holds_inside_its_root_and_outside_every_project() {
        for cwd in [r"C:\work\blog\public", r"D:\scratch"] {
            let mut req = request("php", cwd, &[]);
            req.inherited_site = Some("blog".into());
            req.inherited_root = Some(r"C:\work\blog".into());
            let planned = plan(&req, &tools::all(), &index_with_two_projects(), facts()).unwrap();
            assert_eq!(planned.invocation.program, PathBuf::from(r"C:\php\7.4\php.exe"), "{cwd}");
            assert!(planned.notices.is_empty(), "{cwd}");
        }
    }

    struct StoredRootPassword;
    impl MysqlRootPasswordReader for StoredRootPassword {
        fn root_password(&self, _instance: &ServiceId) -> PortResult<Option<MysqlPassword>> {
            Ok(Some(MysqlPassword::new("s3cret").unwrap()))
        }
    }

    struct NeverAsked;
    impl MysqlRootPasswordReader for NeverAsked {
        fn root_password(&self, instance: &ServiceId) -> PortResult<Option<MysqlPassword>> {
            panic!("no tool asked for the password of {instance}")
        }
    }
    impl MysqlProjectPasswordReader for NeverAsked {
        fn project_password(&self, project: &ProjectId) -> PortResult<Option<MysqlPassword>> {
            panic!("no tool asked for the password of project {project}")
        }
    }

    struct Unreadable;
    impl MysqlRootPasswordReader for Unreadable {
        fn root_password(&self, _instance: &ServiceId) -> PortResult<Option<MysqlPassword>> {
            Err(lampo_domain::PortError::invalid_data("mysql-8.4.root.dpapi", "cannot be decrypted"))
        }
    }

    struct StoredProjectPassword;
    impl MysqlProjectPasswordReader for StoredProjectPassword {
        fn project_password(&self, project: &ProjectId) -> PortResult<Option<MysqlPassword>> {
            assert_eq!(project.as_str(), "shop");
            Ok(Some(MysqlPassword::new("shop-pass").unwrap()))
        }
    }

    /// Only the root password store; a project password is never asked for.
    fn root_only(root: &dyn MysqlRootPasswordReader) -> StoredPasswords<'_> {
        StoredPasswords { root, projects: &NeverAsked }
    }

    fn mysql_invocation(args: &[&str]) -> ShimInvocation {
        mysql_invocation_as(args, "root", None)
    }

    fn mysql_invocation_as(args: &[&str], user: &str, project_account: Option<&str>) -> ShimInvocation {
        let mysql = ShimService {
            service: "mysql-8.4".parse().unwrap(),
            client_dir: PathBuf::from(r"C:mysqlin"),
            host: "127.0.0.1".into(),
            port: Port::of(3306),
            user: Some(user.into()),
            database: None,
            data_dir: None,
            env_prefix: "DB".into(),
            project_account: project_account.map(|p| ProjectId::new(p).unwrap()),
        };
        let target = ShimTarget { services: [(ServiceKind::MYSQL, mysql)].into(), ..target(false) };
        let entry = ShimEntry { site: "shop".parse().unwrap(), root: ProjectRoot::new(r"C:\work\shop").unwrap(), target };
        let index = FakeIndex(ShimIndex { version: 1, default: None, projects: vec![entry] });
        invocation(&request("mysql", r"C:\work\shop", args), &tools::all(), &index).unwrap()
    }

    #[test]
    fn a_stored_root_password_is_read_from_an_option_file_given_first() {
        let planned = mysql_invocation(&["-e", "SELECT 1"]);
        let prepared = with_stored_credentials(planned.clone(), &root_only(&StoredRootPassword)).unwrap();
        let path = prepared._option_file.as_ref().unwrap().path().to_path_buf();
        assert_eq!(prepared.invocation.args[0], format!("--defaults-extra-file={}", path.display()));
        assert_eq!(prepared.invocation.args[1..], planned.args[..], "the rest as planned");
        assert!(!prepared.invocation.args.iter().any(|a| a.contains("s3cret") || a.starts_with("-p") || a.starts_with("--password")));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[client]\npassword=\"s3cret\"\n");
        drop(prepared);
        assert!(!path.exists(), "deleted after the tool");
    }

    #[test]
    fn without_a_stored_password_or_a_request_for_it_the_invocation_is_unchanged() {
        let planned = mysql_invocation(&["-e", "SELECT 1"]);
        let prepared = with_stored_credentials(planned.clone(), &root_only(&NoMysqlRootPasswords)).unwrap();
        assert_eq!((prepared.invocation, prepared._option_file.is_none()), (planned, true));

        let own_account = mysql_invocation(&["-u", "app", "-e", "SELECT 1"]);
        assert_eq!(with_stored_credentials(own_account.clone(), &StoredPasswords { root: &NeverAsked, projects: &NeverAsked }).unwrap().invocation, own_account);
        let php = invocation(&request("php", r"C:\work\shop", &["-v"]), &tools::all(), &index_with_project()).unwrap();
        assert_eq!(with_stored_credentials(php.clone(), &StoredPasswords { root: &NeverAsked, projects: &NeverAsked }).unwrap().invocation, php);
    }

    #[test]
    fn an_unreadable_stored_password_is_a_one_line_failure() {
        let err = with_stored_credentials(mysql_invocation(&[]), &root_only(&Unreadable)).unwrap_err();
        assert!(!err.contains('\n'), "{err}");
        assert!(err.contains("root password Lampo stored for mysql-8.4") && err.contains("cannot be decrypted"), "{err}");
    }

    #[test]
    fn a_project_with_its_own_account_gets_that_accounts_password_and_never_roots() {
        let planned = mysql_invocation_as(&["-e", "SELECT 1"], "lampo_shop", Some("shop"));
        assert!(planned.args.contains(&"--user=lampo_shop".to_string()));
        let prepared = with_stored_credentials(planned, &StoredPasswords { root: &NeverAsked, projects: &StoredProjectPassword }).unwrap();
        let path = prepared._option_file.as_ref().unwrap().path().to_path_buf();
        assert_eq!(prepared.invocation.args[0], format!("--defaults-extra-file={}", path.display()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[client]\npassword=\"shop-pass\"\n");
        assert!(!prepared.invocation.args.iter().any(|a| a.contains("shop-pass")));
    }
}
