//! End-to-end tests of the real `lampo-shim` binary, copied as `php.exe`, in front of a small
//! test executable (`examples/argecho`) that echoes its arguments, stdin size and exit code.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use lampo_domain::php::{ComposerChannel, PhpMinor};
use lampo_domain::project::ProjectRoot;
use lampo_domain::shim::{ShimEntry, ShimIndex, ShimTarget};
use lampo_domain::tcp_port::Port;

struct Sandbox {
    _dir: tempfile::TempDir,
    home: PathBuf,
    shim_dir: PathBuf,
    project: PathBuf,
    php_dir: PathBuf,
}

fn argecho() -> PathBuf {
    let shim = PathBuf::from(env!("CARGO_BIN_EXE_lampo-shim"));
    shim.parent().unwrap().join("examples").join("argecho.exe")
}

fn target(php_dir: &Path) -> ShimTarget {
    let php = PhpMinor::parse("8.3").unwrap();
    ShimTarget {
        php,
        php_dir: php_dir.to_path_buf(),
        php_exe: php_dir.join("php.exe"),
        phprc_dir: Some(PathBuf::from(r"C:\lampo-test-ini")),
        composer_phar: None,
        composer_channel: ComposerChannel::for_php(php),
        services: Default::default(),
        env_vars: Default::default(),
    }
}

impl Sandbox {
    /// A Lampo home with one project served by the test tool installed as `php.exe`.
    fn new(with_php: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let shim_dir = dir.path().join("shims");
        let project = dir.path().join("work").join("shop");
        let php_dir = dir.path().join("php");
        for d in [&shim_dir, &project, &php_dir, &home.join("data").join("bin"), &home.join("config")] {
            std::fs::create_dir_all(d).unwrap();
        }
        if with_php {
            std::fs::copy(argecho(), php_dir.join("php.exe")).expect("build the argecho example first (cargo test builds it)");
        }
        let entry = ShimEntry { site: "shop".parse().unwrap(), root: ProjectRoot::new(project.to_str().unwrap()).unwrap(), target: target(&php_dir) };
        let index = ShimIndex { version: 1, default: None, projects: vec![entry] };
        std::fs::write(home.join("data").join("bin").join("shim-index.json"), serde_json::to_string(&index).unwrap()).unwrap();
        Self { _dir: dir, home, shim_dir, project, php_dir }
    }

    fn install_shim(&self, name: &str) -> PathBuf {
        let path = self.shim_dir.join(name);
        std::fs::copy(env!("CARGO_BIN_EXE_lampo-shim"), &path).unwrap();
        path
    }

    fn command(&self, shim: &Path, cwd: &Path) -> Command {
        let mut c = Command::new(shim);
        c.current_dir(cwd).env("LAMPO_HOME", &self.home).env_remove("LAMPO_PROJECT").env_remove("ARGECHO_EXIT");
        c
    }
}

fn run(mut command: Command, stdin: &[u8]) -> (i32, String, String) {
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.code().unwrap(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn arguments_stdin_env_and_exit_code_pass_through() {
    let sb = Sandbox::new(true);
    let shim = sb.install_shim("php.exe");
    let tricky = ["-r", r#"echo "a b";"#, "two words", "", "it's", r#"say "hi""#, r"C:\dir with space\", r"back\\slash", "100%", "$HOME", "*.php", "caf\u{e9} \u{65e5}\u{672c}", "--", "-v"];
    let mut command = sb.command(&shim, &sb.project.join("app"));
    std::fs::create_dir_all(sb.project.join("app")).unwrap();
    command.args(tricky).env("ARGECHO_EXIT", "7");
    let (code, out, err) = run(command, b"hello stdin");
    assert_eq!(code, 7, "exit code of the tool is forwarded; stderr: {err}");
    let echoed: Vec<String> = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(echoed, tricky);
    assert!(err.contains("stdin=11"), "stdin reaches the tool: {err}");
    assert!(err.contains(r"phprc=C:\lampo-test-ini"), "PHPRC is set for the project: {err}");
}

#[test]
fn exit_code_zero_and_large_codes_are_preserved() {
    let sb = Sandbox::new(true);
    let shim = sb.install_shim("php.exe");
    let (code, _, _) = run(sb.command(&shim, &sb.project), b"");
    assert_eq!(code, 0);
    let mut command = sb.command(&shim, &sb.project);
    command.env("ARGECHO_EXIT", "255");
    assert_eq!(run(command, b"").0, 255);
}

#[test]
fn works_in_any_case_of_the_file_name() {
    let sb = Sandbox::new(true);
    let shim = sb.install_shim("PHP.EXE");
    let (code, out, err) = run(sb.command(&shim, &sb.project), b"");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), "[]");
}

#[test]
fn lampo_project_selects_the_site_regardless_of_the_folder() {
    let sb = Sandbox::new(true);
    let shim = sb.install_shim("php.exe");
    let mut command = sb.command(&shim, sb.project.parent().unwrap().parent().unwrap());
    command.env("LAMPO_PROJECT", "shop");
    let (code, _, err) = run(command, b"");
    assert_eq!(code, 0, "{err}");
}

#[test]
fn outside_any_project_exits_127_with_one_line() {
    let sb = Sandbox::new(true);
    let shim = sb.install_shim("php.exe");
    let outside = sb.home.clone();
    let (code, out, err) = run(sb.command(&shim, &outside), b"");
    assert_eq!((code, out.as_str()), (127, ""));
    assert_eq!(err.lines().count(), 1, "{err}");
    assert!(err.starts_with("lampo-shim: no Lampo project"), "{err}");
}

#[test]
fn missing_tool_binary_exits_127_with_one_line() {
    let sb = Sandbox::new(false);
    let shim = sb.install_shim("php.exe");
    let (code, _, err) = run(sb.command(&shim, &sb.project), b"");
    assert_eq!(code, 127);
    assert_eq!(err.lines().count(), 1, "{err}");
    assert!(err.starts_with("lampo-shim: cannot start"), "{err}");
    assert!(err.contains(sb.php_dir.join("php.exe").to_str().unwrap()), "{err}");
}

#[test]
fn unknown_shim_name_exits_127() {
    let sb = Sandbox::new(true);
    let shim = sb.install_shim("notatool.exe");
    let (code, _, err) = run(sb.command(&shim, &sb.project), b"");
    assert_eq!(code, 127);
    assert!(err.contains("not a Lampo shim name"), "{err}");
}

#[test]
fn works_without_the_app_and_without_any_other_lampo_file() {
    // The sandbox has no settings, no projects dir, no pipe: only the shim index.
    let sb = Sandbox::new(true);
    let shim = sb.install_shim("php.exe");
    assert_eq!(run(sb.command(&shim, &sb.project), b"").0, 0);
    assert!(!sb.home.join("config").join("settings.toml").exists());
}

#[test]
fn startup_overhead_is_small() {
    let sb = Sandbox::new(true);
    let shim = sb.install_shim("php.exe");
    let direct = sb.php_dir.join("php.exe");
    let time = |program: &Path, via_shim: bool| {
        let started = std::time::Instant::now();
        let runs = 20;
        for _ in 0..runs {
            let mut c = if via_shim { sb.command(program, &sb.project) } else { Command::new(program) };
            c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            assert!(c.status().unwrap().success());
        }
        started.elapsed() / runs
    };
    let (shimmed, plain) = (time(&shim, true), time(&direct, false));
    eprintln!("shim overhead: {shimmed:?} per run via shim vs {plain:?} direct (+{:?})", shimmed.saturating_sub(plain));
    // Generous bound: debug build on a busy machine. The measured figure is printed above.
    assert!(shimmed.saturating_sub(plain) < std::time::Duration::from_millis(250));
}

/// The project `shop` with a MySQL instance whose client is the test tool installed as
/// `mysql.exe`, database `shop_feature` and data dir `<sandbox>\mysql-data` (with the database's
/// folder only when `database_exists`). Returns the installed `mysql` shim.
fn install_mysql(sb: &Sandbox, database_exists: bool) -> PathBuf {
    use lampo_domain::service::ServiceKind;
    use lampo_domain::shim::ShimService;
    std::fs::copy(argecho(), sb.php_dir.join("mysql.exe")).unwrap();
    let data_dir = sb.home.join("mysql-data");
    std::fs::create_dir_all(&data_dir).unwrap();
    if database_exists {
        std::fs::create_dir(data_dir.join("shop_feature")).unwrap();
    }
    let mysql = ShimService {
        service: "mysql-8.4".parse().unwrap(),
        client_dir: sb.php_dir.clone(),
        host: "127.0.0.1".into(),
        port: Port::of(3307),
        user: Some("root".into()),
        database: Some("shop_feature".into()),
        data_dir: Some(data_dir),
        env_prefix: "DB".into(),
    };
    let mut target = target(&sb.php_dir);
    target.services = [(ServiceKind::MYSQL, mysql)].into();
    let entry = ShimEntry { site: "shop".parse().unwrap(), root: ProjectRoot::new(sb.project.to_str().unwrap()).unwrap(), target };
    let index = ShimIndex { version: 2, default: None, projects: vec![entry] };
    std::fs::write(sb.home.join("data").join("bin").join("shim-index.json"), serde_json::to_string(&index).unwrap()).unwrap();
    sb.install_shim("mysql.exe")
}

#[test]
fn mysql_gets_the_sites_instance_and_database_and_forwards_stdin_and_exit_code() {
    let sb = Sandbox::new(false);
    let shim = install_mysql(&sb, true);

    let mut command = sb.command(&shim, &sb.project);
    command.args(["-N", "-B", "-e", "SHOW TABLES"]).env("ARGECHO_EXIT", "1");
    let (code, out, err) = run(command, b"SELECT 1;");
    assert_eq!(code, 1, "the client's exit code is forwarded; stderr: {err}");
    let echoed: Vec<String> = serde_json::from_str(out.trim()).unwrap();
    let expected = ["--host=127.0.0.1", "--port=3307", "--user=root", "--default-character-set=utf8mb4", "--database=shop_feature", "-N", "-B", "-e", "SHOW TABLES"];
    assert_eq!(echoed, expected);
    assert!(err.contains("stdin=9"), "stdin reaches the client: {err}");

    let mut command = sb.command(&shim, &sb.project);
    command.args(["--database=", "-e", "SHOW DATABASES"]);
    let (_, out, _) = run(command, b"");
    let echoed: Vec<String> = serde_json::from_str(out.trim()).unwrap();
    assert!(!echoed.iter().any(|a| a == "--database=shop_feature"), "an explicit (empty) database wins: {echoed:?}");
}

/// `mysql -e "CREATE DATABASE ..."` (or a dump import) before the site's database exists must
/// not fail on `ERROR 1049 Unknown database`: the shim connects without a default database.
#[test]
fn mysql_connects_without_the_sites_database_until_it_exists() {
    let sb = Sandbox::new(false);
    let shim = install_mysql(&sb, false);
    let defaults = ["--host=127.0.0.1", "--port=3307", "--user=root", "--default-character-set=utf8mb4"];

    let mut command = sb.command(&shim, &sb.project);
    command.args(["-e", "CREATE DATABASE shop_feature"]).env("ARGECHO_EXIT", "3").env_remove("LAMPO_QUIET");
    let (code, out, err) = run(command, b"");
    assert_eq!(code, 3, "the client's exit code is forwarded; stderr: {err}");
    let echoed: Vec<String> = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(echoed, [&defaults[..], &["-e", "CREATE DATABASE shop_feature"]].concat());
    assert!(err.starts_with("lampo-shim: database 'shop_feature' does not exist yet, connecting without a default database\n"), "{err}");

    std::fs::create_dir(sb.home.join("mysql-data").join("shop_feature")).unwrap();
    let mut command = sb.command(&shim, &sb.project);
    command.args(["-e", "SHOW TABLES"]).env_remove("LAMPO_QUIET");
    let (_, out, err) = run(command, b"");
    let echoed: Vec<String> = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(echoed, [&defaults[..], &["--database=shop_feature", "-e", "SHOW TABLES"]].concat(), "once created it is selected again");
    assert!(!err.contains("lampo-shim:"), "{err}");
}

/// A second project, `blog`, next to `shop`, with its own php.ini dir (echoed as `phprc=`).
fn add_blog(sb: &Sandbox) -> PathBuf {
    let blog = sb.project.parent().unwrap().join("blog");
    std::fs::create_dir_all(&blog).unwrap();
    let index_file = sb.home.join("data").join("bin").join("shim-index.json");
    let mut index: ShimIndex = serde_json::from_str(&std::fs::read_to_string(&index_file).unwrap()).unwrap();
    let target = ShimTarget { phprc_dir: Some(PathBuf::from(r"C:\blog-test-ini")), ..target(&sb.php_dir) };
    index.projects.push(ShimEntry { site: "blog".parse().unwrap(), root: ProjectRoot::new(blog.to_str().unwrap()).unwrap(), target });
    std::fs::write(index_file, serde_json::to_string(&index).unwrap()).unwrap();
    blog
}

/// In the terminal of `blog` (`LAMPO_PROJECT=blog`), `cd ..\shop; php ...` runs with shop's
/// settings and says so in one line.
#[test]
fn lampo_project_of_another_project_gives_way_to_the_folder() {
    let sb = Sandbox::new(true);
    let blog = add_blog(&sb);
    let shim = sb.install_shim("php.exe");
    let inherited = |command: &mut Command| {
        command.env("LAMPO_PROJECT", "blog").env("LAMPO_PROJECT_ROOT", &blog).env_remove("LAMPO_QUIET");
    };

    let mut command = sb.command(&shim, &sb.project);
    inherited(&mut command);
    let (code, _, err) = run(command, b"");
    assert_eq!(code, 0, "{err}");
    assert!(err.contains(r"phprc=C:\lampo-test-ini"), "shop's settings: {err}");
    assert!(err.starts_with("lampo-shim: LAMPO_PROJECT=blog ignored: this folder belongs to shop"), "{err}");

    let mut command = sb.command(&shim, &blog);
    inherited(&mut command);
    let (_, _, err) = run(command, b"");
    assert!(err.contains(r"phprc=C:\blog-test-ini") && !err.contains("ignored"), "inside its root it holds: {err}");

    let mut command = sb.command(&shim, &sb.project);
    inherited(&mut command);
    command.env("LAMPO_QUIET", "1");
    let (_, _, err) = run(command, b"");
    assert!(!err.contains("lampo-shim:"), "LAMPO_QUIET hides the note: {err}");
}

#[test]
fn a_junction_to_the_project_resolves_the_project() {
    let sb = Sandbox::new(true);
    let shim = sb.install_shim("php.exe");
    let link = sb.home.parent().unwrap().join("shop-link");
    let made = Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(&link).arg(&sb.project).output().unwrap();
    assert!(made.status.success(), "mklink /J: {}", String::from_utf8_lossy(&made.stderr));
    let (code, _, err) = run(sb.command(&shim, &link), b"");
    assert_eq!(code, 0, "the final path of the junction is the project: {err}");
    assert!(err.contains(r"phprc=C:\lampo-test-ini"), "{err}");
}

/// An IDE or CI runner that stops `php artisan serve` kills the shim, not PHP: the tool must die
/// with it instead of running on, orphaned.
#[test]
fn killing_the_shim_kills_the_tool() {
    use std::io::{BufRead, BufReader, Read};
    use std::sync::mpsc;
    use std::time::Duration;

    let sb = Sandbox::new(true);
    let shim = sb.install_shim("php.exe");
    let mut command = sb.command(&shim, &sb.project);
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    // Held open until the end: the tool blocks reading it, like a server that never returns.
    let stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    stdout.read_line(&mut first).unwrap();
    assert_eq!(first.trim(), "[]", "the tool is running");

    child.kill().unwrap();
    child.wait().unwrap();
    // The tool writes to the same stdout pipe: the pipe reaches EOF only once the tool is gone.
    let (gone, closed) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = stdout.read_to_end(&mut Vec::new());
        let _ = gone.send(());
    });
    let tool_died = closed.recv_timeout(Duration::from_secs(10)).is_ok();
    drop(stdin);
    assert!(tool_died, "the tool outlived the shim");
}
