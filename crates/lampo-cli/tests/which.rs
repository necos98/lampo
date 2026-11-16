//! End-to-end tests of the real `lampo which` and `lampo completions`, with a Lampo home holding
//! only a shim index: `shop` (PHP 8.3, with Composer) and a global default (PHP 8.4, without).

use std::path::{Path, PathBuf};
use std::process::Command;

use lampo_domain::php::{ComposerChannel, PhpMinor};
use lampo_domain::project::ProjectRoot;
use lampo_domain::shim::{ShimEntry, ShimIndex, ShimTarget};
use serde_json::Value;

struct Sandbox {
    _dir: tempfile::TempDir,
    home: PathBuf,
    shop: PathBuf,
    elsewhere: PathBuf,
}

fn target(dir: &Path, minor: &str, composer: bool) -> ShimTarget {
    let php = PhpMinor::parse(minor).unwrap();
    let php_dir = dir.join(format!("php-{minor}"));
    ShimTarget {
        php,
        php_exe: php_dir.join("php.exe"),
        php_dir,
        phprc_dir: None,
        composer_phar: composer.then(|| dir.join("composer").join("composer.phar")),
        composer_channel: ComposerChannel::for_php(php),
        services: Default::default(),
        env_vars: Default::default(),
    }
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let shop = dir.path().join("work").join("shop");
        let elsewhere = dir.path().join("elsewhere");
        for d in [&shop, &elsewhere, &home.join("data").join("bin"), &home.join("config")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let entry = ShimEntry { site: "shop".parse().unwrap(), root: ProjectRoot::new(shop.to_str().unwrap()).unwrap(), target: target(dir.path(), "8.3", true) };
        let index = ShimIndex { default: Some(target(dir.path(), "8.4", false)), projects: vec![entry], ..Default::default() };
        std::fs::write(home.join("data").join("bin").join("shim-index.json"), serde_json::to_string(&index).unwrap()).unwrap();
        Self { _dir: dir, home, shop, elsewhere }
    }

    /// `lampo <args>` in `cwd`, with `LAMPO_PROJECT` set to `inherited` (else removed).
    fn lampo(&self, cwd: &Path, inherited: Option<&str>, args: &[&str]) -> (i32, String, String) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lampo"));
        command.args(args).arg("--no-start").current_dir(cwd).env("LAMPO_HOME", &self.home).env_remove("LAMPO_PROJECT_ROOT").env_remove("LAMPO_QUIET");
        match inherited {
            Some(site) => command.env("LAMPO_PROJECT", site),
            None => command.env_remove("LAMPO_PROJECT"),
        };
        let out = command.output().unwrap();
        (out.status.code().unwrap(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    }

    fn which_json(&self, cwd: &Path, inherited: Option<&str>, args: &[&str]) -> (i32, Value) {
        let mut all = vec!["which"];
        all.extend_from_slice(args);
        all.push("--json");
        let (code, out, err) = self.lampo(cwd, inherited, &all);
        let doc = serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}: {out:?} {err}"));
        (code, doc)
    }
}

#[test]
fn which_names_the_site_the_binary_and_the_reason() {
    let s = Sandbox::new();
    let inside = s.shop.join("public");
    std::fs::create_dir_all(&inside).unwrap();

    let (code, doc) = s.which_json(&inside, None, &["php"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!((doc["reason"].as_str(), doc["resolution"]["site"].as_str(), doc["phpVersion"].as_str()), (Some("directory"), Some("shop"), Some("8.3")));
    assert!(doc["binary"].as_str().unwrap().ends_with(r"php-8.3\php.exe"), "{doc}");

    let (_, doc) = s.which_json(&s.elsewhere, Some("shop"), &["composer"]);
    assert_eq!((doc["reason"].as_str(), doc["resolution"]["site"].as_str()), (Some("lampo-project"), Some("shop")));
    assert!(doc["defaultArgs"][0].as_str().unwrap().ends_with("composer.phar"), "{doc}");

    let (_, doc) = s.which_json(&s.elsewhere, None, &["php", "--project", "shop"]);
    assert_eq!(doc["reason"], "project-flag");

    let (_, doc) = s.which_json(&s.elsewhere, None, &["php"]);
    assert_eq!((doc["reason"].as_str(), doc["resolution"]["site"].is_null(), doc["phpVersion"].as_str()), (Some("global-default"), true, Some("8.4")));
}

#[test]
fn which_failures_keep_the_documented_exit_codes() {
    let s = Sandbox::new();
    let (code, doc) = s.which_json(&s.elsewhere, None, &["composer"]);
    assert_eq!((code, doc["error"]["code"].as_str()), (10, Some("package-missing")), "the global default has no Composer");
    let (code, out, err) = s.lampo(&s.elsewhere, None, &["which", "node"]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(err.contains("php, composer, mysql, mysqldump, redis-cli"), "{err}");
}

#[test]
fn which_prints_a_readable_answer() {
    let s = Sandbox::new();
    let (code, out, _) = s.lampo(&s.shop, None, &["which", "composer"]);
    assert_eq!(code, 0, "{out}");
    let first = out.lines().next().unwrap();
    assert!(first.starts_with("composer: ") && first.ends_with(r"php-8.3\php.exe"), "{out}");
    assert!(out.contains("reason:  the current folder is inside"), "{out}");
}

#[test]
fn completions_print_a_powershell_script() {
    let s = Sandbox::new();
    let (code, out, err) = s.lampo(&s.elsewhere, None, &["completions", "powershell"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.contains("Register-ArgumentCompleter -Native -CommandName 'lampo'"), "{out}");
    assert!(out.contains("'which'") && out.contains("'completions'"), "{out}");
}
