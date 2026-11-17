//! The real `lampo-hosts-helper` binary in its Defender mode, run non-elevated with requests it
//! must refuse before reaching Defender. No valid folder is ever passed, so nothing changes even
//! when the tests run as an administrator.

use std::process::Command;

use lampo_infra_web::defender::helper_exit;

const HELPER: &str = env!("CARGO_BIN_EXE_lampo-hosts-helper");

fn run_helper(args: &[&str]) -> i32 {
    Command::new(HELPER).args(args).status().unwrap().code().unwrap()
}

fn exclusion(change: &str, path: &str) -> i32 {
    run_helper(&["--defender-exclusion", change, "--path", path])
}

fn profile() -> String {
    std::env::var("USERPROFILE").expect("USERPROFILE")
}

#[test]
fn folders_other_than_lampos_mysql_data_are_refused() {
    let profile = profile();
    for path in [
        format!(r"{profile}\AppData\Local\Lampo\mysql2"),
        format!(r"{profile}\AppData\Local\Lampo\mysql\8.4"),
        format!(r"{profile}\AppData\Local\Lampo"),
        format!(r"{profile}\AppData\Local\Lampo\packages"),
        format!(r"{profile}\AppData\Local\Lampo\..\Lampo\mysql"),
        format!(r"{profile}\AppData\Local\Lampo\mysql\"),
        format!(r"{profile}\AppData\Local\Lampo\mysq*"),
        format!(r"\\?\{profile}\AppData\Local\Lampo\mysql"),
        r"\\localhost\c$\Users\someone\AppData\Local\Lampo\mysql".to_string(),
        r"AppData\Local\Lampo\mysql".to_string(),
        r"%LOCALAPPDATA%\Lampo\mysql".to_string(),
        r"C:\Windows".to_string(),
        r"C:\".to_string(),
    ] {
        assert_eq!(exclusion("add", &path), helper_exit::INVALID_TARGET, "add {path}");
        assert_eq!(exclusion("remove", &path), helper_exit::INVALID_TARGET, "remove {path}");
    }
}

#[test]
fn malformed_command_lines_are_refused() {
    let folder = format!(r"{}\AppData\Local\Lampo\mysql", profile());
    for args in [
        vec!["--defender-exclusion"],
        vec!["--defender-exclusion", "add"],
        vec!["--defender-exclusion", "exclude", "--path", &folder],
        vec!["--defender-exclusion", "add", "--path", &folder, "--extension", ".ibd"],
        vec!["--defender-exclusion", "add", "--process", "mysqld.exe"],
    ] {
        assert_eq!(run_helper(&args), helper_exit::BAD_ARGUMENTS, "{args:?}");
    }
}
