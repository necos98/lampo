//! End-to-end tests of the real `lampo exec`, with a Lampo home holding only a shim index whose
//! global default serves every folder, and the `examples/argecho` test tool as the child.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use lampo_domain::php::{ComposerChannel, PhpMinor};
use lampo_domain::shim::{ShimIndex, ShimTarget};

fn argecho() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lampo")).parent().unwrap().join("examples").join("argecho.exe")
}

/// A Lampo home whose shim index has only a global default.
fn home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("data").join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    let php = PhpMinor::parse("8.3").unwrap();
    let php_dir = dir.path().join("php");
    let default = ShimTarget {
        php,
        php_exe: php_dir.join("php.exe"),
        php_dir,
        phprc_dir: None,
        composer_phar: None,
        composer_channel: ComposerChannel::for_php(php),
        services: Default::default(),
        env_vars: Default::default(),
    };
    let index = ShimIndex { default: Some(default), ..Default::default() };
    std::fs::write(bin.join("shim-index.json"), serde_json::to_string(&index).unwrap()).unwrap();
    dir
}

/// An IDE that stops `lampo exec -- php artisan serve` kills `lampo`, not the child: the child
/// must die with it.
#[test]
fn killing_lampo_exec_kills_the_child() {
    let home = home();
    let mut lampo = Command::new(env!("CARGO_BIN_EXE_lampo"))
        .args(["exec", "--"])
        .arg(argecho())
        .current_dir(home.path())
        .env("LAMPO_HOME", home.path())
        .env_remove("LAMPO_PROJECT")
        .env_remove("LAMPO_PROJECT_ROOT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Held open until the end: the child blocks reading it.
    let stdin = lampo.stdin.take().unwrap();
    let mut stdout = BufReader::new(lampo.stdout.take().unwrap());
    let mut first = String::new();
    stdout.read_line(&mut first).unwrap();
    assert_eq!(first.trim(), "[]", "the child is running");

    lampo.kill().unwrap();
    lampo.wait().unwrap();
    // The child writes to the same stdout pipe: EOF arrives only once the child is gone.
    let (gone, closed) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = stdout.read_to_end(&mut Vec::new());
        let _ = gone.send(());
    });
    let child_died = closed.recv_timeout(Duration::from_secs(10)).is_ok();
    drop(stdin);
    assert!(child_died, "the child outlived `lampo exec`");
}

/// A child that exits on its own keeps the usual behaviour: arguments unchanged (`--json`
/// included), exit code forwarded.
#[test]
fn exit_code_of_the_child_is_forwarded() {
    let home = home();
    let out = Command::new(env!("CARGO_BIN_EXE_lampo"))
        .args(["exec", "--"])
        .arg(argecho())
        .args(["a b", "--json"])
        .current_dir(home.path())
        .env("LAMPO_HOME", home.path())
        .env("ARGECHO_EXIT", "5")
        .env_remove("LAMPO_PROJECT")
        .env_remove("LAMPO_PROJECT_ROOT")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(5), "{}", String::from_utf8_lossy(&out.stderr));
    let echoed: Vec<String> = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(echoed, ["a b", "--json"]);
}

/// `--json` among the child's arguments is the child's: errors of `lampo exec` stay plain text.
#[test]
fn json_flag_after_exec_belongs_to_the_child() {
    let home = home();
    let out = Command::new(env!("CARGO_BIN_EXE_lampo"))
        .args(["exec", "lampo-test-no-such-tool", "x", "--json"])
        .current_dir(home.path())
        .env("LAMPO_HOME", home.path())
        .env_remove("LAMPO_PROJECT")
        .env_remove("LAMPO_PROJECT_ROOT")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "", "no JSON document on stdout");
    assert!(String::from_utf8_lossy(&out.stderr).contains("`lampo-test-no-such-tool` not found"), "{}", String::from_utf8_lossy(&out.stderr));
}

/// `lampo shims uninstall` takes away the shims, not the shim index the CLI resolves the site
/// with: `lampo exec` keeps working afterwards (as does "Apri terminale", `lampo -p <site> exec`).
#[test]
fn shims_uninstall_keeps_lampo_exec_working() {
    let home = home();
    let bin = home.path().join("data").join("bin");
    for copy in ["php.exe", "composer.exe", "lampo.exe"] {
        std::fs::write(bin.join(copy), b"copy").unwrap();
    }
    let lampo = |args: &[&OsStr]| {
        Command::new(env!("CARGO_BIN_EXE_lampo"))
            .args(args)
            .current_dir(home.path())
            .env("LAMPO_HOME", home.path())
            .env_remove("LAMPO_PROJECT")
            .env_remove("LAMPO_PROJECT_ROOT")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let out = lampo(&["shims".as_ref(), "uninstall".as_ref()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!bin.join("php.exe").exists() && !bin.join("lampo.exe").exists(), "the copies are gone");

    let out = lampo(&["exec".as_ref(), "--".as_ref(), argecho().as_os_str(), "ok".as_ref()]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let echoed: Vec<String> = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(echoed, ["ok"]);
}
