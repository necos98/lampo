//! The real `lampo-hosts-helper` binary in its Defender mode, run non-elevated with requests it
//! must refuse before reaching Defender. No valid target is ever passed, so nothing changes even
//! when the tests run as an administrator.

use std::process::Command;

use lampo_infra_web::defender::helper_exit;

const HELPER: &str = env!("CARGO_BIN_EXE_lampo-hosts-helper");

fn run_helper(args: &[&str]) -> i32 {
    Command::new(HELPER).args(args).status().unwrap().code().unwrap()
}

fn exclusion(change: &str, flag: &str, path: &str) -> i32 {
    run_helper(&["--defender-exclusion", change, flag, path])
}

fn profile() -> String {
    std::env::var("USERPROFILE").expect("USERPROFILE")
}

fn assert_refused(flag: &str, paths: &[String]) {
    for path in paths {
        assert_eq!(exclusion("add", flag, path), helper_exit::INVALID_TARGET, "add {flag} {path}");
        assert_eq!(exclusion("remove", flag, path), helper_exit::INVALID_TARGET, "remove {flag} {path}");
    }
}

#[test]
fn folders_other_than_lampos_mysql_data_are_refused() {
    let profile = profile();
    assert_refused(
        "--path",
        &[
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
        ],
    );
}

#[test]
fn executables_other_than_lampos_servers_are_refused() {
    let profile = profile();
    let packages = format!(r"{profile}\AppData\Local\Lampo\packages");
    assert_refused(
        "--process",
        &[
            format!(r"{packages}\php-8.3\php.exe"),
            format!(r"{packages}\mysql-8.4\bin\mysqld.exe"),
            format!(r"{packages}\apache-2.4\bin\ab.exe"),
            format!(r"{packages}\apache-2.4\httpd.exe"),
            format!(r"{packages}\php-redis-6.3-php8.3\php-cgi.exe"),
            format!(r"{packages}\php-*\php-cgi.exe"),
            format!(r"{packages}\..\packages\php-8.3\php-cgi.exe"),
            format!(r"{profile}\AppData\Local\Lampo\php-cgi.exe"),
            format!(r"\\?\{packages}\php-8.3\php-cgi.exe"),
            r"\\localhost\c$\Users\someone\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe".to_string(),
            r"C:\Users\nobody-registered\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe".to_string(),
            r"%LOCALAPPDATA%\Lampo\packages\php-8.3\php-cgi.exe".to_string(),
            "php-cgi.exe".to_string(),
            "mysqld.exe".to_string(),
            r"C:\Windows\System32\cmd.exe".to_string(),
        ],
    );
}

#[test]
fn malformed_command_lines_are_refused() {
    let folder = format!(r"{}\AppData\Local\Lampo\mysql", profile());
    for args in [
        vec!["--defender-exclusion"],
        vec!["--defender-exclusion", "add"],
        vec!["--defender-exclusion", "exclude", "--path", &folder],
        vec!["--defender-exclusion", "add", "--path", &folder, "--extension", ".ibd"],
        vec!["--defender-exclusion", "add", "--process"],
        vec!["--defender-exclusion", "add", "--process", "php-cgi.exe", "--path", &folder],
    ] {
        assert_eq!(run_helper(&args), helper_exit::BAD_ARGUMENTS, "{args:?}");
    }
}
