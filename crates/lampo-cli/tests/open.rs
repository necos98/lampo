//! End-to-end tests of the real `lampo open`, with a Lampo home holding one project, `shop`
//! (stored and in the shim index, as the app leaves them), whose folder has been deleted. Only
//! the paths that start nothing are run here: a launch would open Explorer, VS Code or the
//! browser on the machine running the tests.

use std::path::{Path, PathBuf};
use std::process::Command;

use lampo_domain::address::SiteAddress;
use lampo_domain::php::{ComposerChannel, PhpMinor};
use lampo_domain::ports::ProjectRepository;
use lampo_domain::project::{DisplayName, Project, ProjectEnvironment, ProjectId, ProjectRoot};
use lampo_domain::shim::{ShimEntry, ShimIndex, ShimTarget};
use lampo_infra_store::TomlProjectRepository;
use serde_json::Value;

struct Sandbox {
    _dir: tempfile::TempDir,
    home: PathBuf,
    /// The registered folder of `shop`, which does not exist.
    shop: PathBuf,
    elsewhere: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let shop = dir.path().join("work").join("shop");
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let paths = lampo_infra_store::paths::resolve_with(Some(home.clone().into())).unwrap();
        let php = PhpMinor::parse("8.3").unwrap();
        let root = ProjectRoot::new(shop.to_str().unwrap()).unwrap();
        let project =
            Project::new(ProjectId::new("shop").unwrap(), DisplayName::new("Shop").unwrap(), root.clone(), vec![SiteAddress::localhost("shop").unwrap()], ProjectEnvironment::minimal(php))
                .unwrap();
        TomlProjectRepository::new(paths.clone()).save(&project).unwrap();
        let php_dir = dir.path().join("php-8.3");
        let target = ShimTarget {
            php,
            php_exe: php_dir.join("php.exe"),
            php_dir,
            phprc_dir: None,
            composer_phar: None,
            composer_channel: ComposerChannel::for_php(php),
            services: Default::default(),
            env_vars: Default::default(),
        };
        let index = ShimIndex { projects: vec![ShimEntry { site: "shop".parse().unwrap(), root, target }], ..Default::default() };
        let index_file = paths.shim_index_file();
        std::fs::create_dir_all(index_file.parent().unwrap()).unwrap();
        std::fs::write(index_file, serde_json::to_string(&index).unwrap()).unwrap();
        Self { _dir: dir, home, shop, elsewhere }
    }

    /// `lampo open <args> --json --no-start` in `cwd`, outside every inherited site.
    fn open_json(&self, cwd: &Path, args: &[&str]) -> (i32, Value, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_lampo"))
            .arg("open")
            .args(args)
            .args(["--json", "--no-start"])
            .current_dir(cwd)
            .env("LAMPO_HOME", &self.home)
            .env_remove("LAMPO_PROJECT")
            .env_remove("LAMPO_PROJECT_ROOT")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let doc = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("{e}: {stdout:?}"));
        (out.status.code().unwrap(), doc, String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

#[test]
fn outside_every_project_is_exit_6_with_a_hint() {
    let s = Sandbox::new();
    let (code, doc, _) = s.open_json(&s.elsewhere, &["--code"]);
    assert_eq!((code, doc["error"]["code"].as_str()), (6, Some("project-not-resolved")), "{doc}");
    assert!(doc["error"]["hint"].as_str().unwrap().contains("--project"), "{doc}");
}

#[test]
fn a_missing_folder_is_exit_4_and_names_it() {
    let s = Sandbox::new();
    for app in ["--code", "--explorer"] {
        let (code, doc, _) = s.open_json(&s.elsewhere, &[app, "--project", "shop"]);
        assert_eq!((code, doc["error"]["code"].as_str()), (4, Some("validation")), "{app}: {doc}");
        assert_eq!(doc["error"]["details"]["folder"].as_str(), s.shop.to_str(), "{app}");
    }
}

#[test]
fn an_unknown_project_is_not_found() {
    let s = Sandbox::new();
    let (code, doc, _) = s.open_json(&s.elsewhere, &["--project", "ghost"]);
    assert_eq!((code, doc["error"]["code"].as_str()), (3, Some("not-found")), "{doc}");
}

#[test]
fn two_apps_at_once_are_a_usage_error() {
    let s = Sandbox::new();
    let (code, doc, _) = s.open_json(&s.elsewhere, &["--code", "--browser"]);
    assert_eq!((code, doc["error"]["code"].as_str()), (2, Some("usage")), "{doc}");
}
