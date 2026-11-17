//! New-project setup: git clone and the Lampo-managed Laravel Installer, with cleanup.

use std::path::PathBuf;
use std::sync::Arc;

use lampo_domain::package::PackageKind;
use lampo_domain::ports::ScaffoldFiles;
use lampo_domain::progress::CancelFlag;
use lampo_domain::scaffold::PathState;

use crate::dto::*;
use crate::error::{AppResult, ErrorCode};
use crate::testing::{FakeLocator, FakeScaffoldFiles, FakeStreamingRunner, RecordingProgress, ScriptedCommand, World};
use crate::use_cases::{ScaffoldProject, ScaffoldProjectDeps};

const GIT: &str = "C:\\Program Files\\Git\\cmd\\git.exe";
const SCRIPT_84: &str = "C:\\data\\tools\\laravel-installer\\8.4\\vendor\\laravel\\installer\\bin\\laravel";

struct Setup {
    world: World,
    files: Arc<FakeScaffoldFiles>,
    runner: Arc<FakeStreamingRunner>,
    locator: Arc<FakeLocator>,
    progress: RecordingProgress,
}

impl Setup {
    fn new(world: World) -> Self {
        let files = Arc::new(FakeScaffoldFiles::default());
        files.dir("C:\\sites");
        let runner = FakeStreamingRunner::new(files.clone());
        let locator = Arc::new(FakeLocator::default());
        locator.0.lock().unwrap().insert("git.exe".into(), vec![PathBuf::from(GIT)]);
        Self { world, files, runner, locator, progress: RecordingProgress::default() }
    }

    fn use_case(&self) -> ScaffoldProject {
        ScaffoldProject::new(ScaffoldProjectDeps {
            files: self.files.clone(),
            runner: self.runner.clone(),
            locator: self.locator.clone(),
            installed: self.world.installed.clone(),
            php_ini: self.world.ini.clone(),
            paths: self.world.paths.clone(),
        })
    }

    async fn run(&self, request: ScaffoldProjectRequest, cancel: &CancelFlag) -> AppResult<ScaffoldProjectResultDto> {
        self.use_case().execute(request, &self.progress, cancel).await
    }

    fn messages(&self) -> Vec<String> {
        self.progress.0.lock().unwrap().iter().filter_map(|p| p.message.clone()).collect()
    }
}

fn clone_request(url: &str, folder: Option<&str>) -> ScaffoldProjectRequest {
    ScaffoldProjectRequest { parent_dir: "C:\\sites".into(), folder_name: folder.map(str::to_string), source: ScaffoldSourceDto::GitClone { url: url.into(), branch: None } }
}

fn laravel_request(name: &str, php: Option<&str>) -> ScaffoldProjectRequest {
    ScaffoldProjectRequest {
        parent_dir: "C:\\sites".into(),
        folder_name: Some(name.into()),
        source: ScaffoldSourceDto::Laravel { php_version: php.map(str::to_string), starter_kit: StarterKitDto::Vue, testing: TestingFrameworkDto::Pest, git: false, npm: false },
    }
}

#[tokio::test]
async fn git_clone_streams_cleaned_output_and_returns_the_folder() {
    let s = Setup::new(World::with_basics());
    s.runner.on(ScriptedCommand {
        matches: " clone ".into(),
        lines: vec!["Cloning into 'shop'...".into(), "\u{1b}[32mReceiving objects: 100% (13/13)\u{1b}[0m".into(), "   ".into()],
        exit: Some(0),
        creates_dirs: vec!["C:\\sites\\shop".into()],
        ..Default::default()
    });
    let mut request = clone_request("git@github.com:acme/shop.git", None);
    request.source = ScaffoldSourceDto::GitClone { url: "git@github.com:acme/shop.git".into(), branch: Some(" develop ".into()) };
    let result = s.run(request, &CancelFlag::new()).await.unwrap();
    assert_eq!(result.folder, "C:\\sites\\shop");
    assert_eq!(result.php_version, None);

    let calls = s.runner.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].program, PathBuf::from(GIT));
    assert_eq!(calls[0].args, ["clone", "--progress", "--branch", "develop", "--", "git@github.com:acme/shop.git", "C:\\sites\\shop"]);
    assert_eq!(calls[0].working_dir.as_deref(), Some(std::path::Path::new("C:\\sites")));
    assert!(calls[0].env.contains(&("GIT_TERMINAL_PROMPT".into(), "0".into())));
    assert_eq!(s.messages(), ["> git clone", "Cloning into 'shop'...", "Receiving objects: 100% (13/13)"]);
}

#[tokio::test]
async fn git_missing_is_a_package_missing_error() {
    let s = Setup::new(World::with_basics());
    s.locator.0.lock().unwrap().clear();
    let err = s.run(clone_request("https://github.com/acme/shop.git", None), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PackageMissing);
    assert_eq!(err.details.as_ref().unwrap()["tool"], "git");
    assert!(err.message.contains("git-scm.com"));
    assert!(s.runner.calls.lock().unwrap().is_empty());
}

#[test]
fn parent_dir_exists_only_for_an_existing_absolute_folder() {
    let s = Setup::new(World::with_basics());
    s.files.file("C:\\sites\\readme.txt", "");
    let scaffold = s.use_case();

    assert!(scaffold.parent_dir_exists("C:\\sites"));
    assert!(scaffold.parent_dir_exists("C:\\sites\\"));
    for missing in ["D:\\nowhere", "C:\\sites\\readme.txt", "sites", "", "   "] {
        assert!(!scaffold.parent_dir_exists(missing), "{missing:?}");
    }
}

#[tokio::test]
async fn destination_must_not_exist_and_parent_must() {
    let s = Setup::new(World::with_basics());
    s.files.dir("C:\\sites\\shop");
    s.files.file("C:\\sites\\shop\\index.php", "<?php");
    let err = s.run(clone_request("https://github.com/acme/shop.git", None), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.message.contains("already exists"), "{}", err.message);

    // An empty folder is fine for a clone, not for the Laravel Installer.
    s.files.dir("C:\\sites\\empty");
    s.run(clone_request("https://github.com/acme/shop.git", Some("empty")), &CancelFlag::new()).await.unwrap();
    let err = s.run(laravel_request("empty", None), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);

    let mut request = clone_request("https://github.com/acme/shop.git", None);
    request.parent_dir = "D:\\nowhere".into();
    assert_eq!(s.run(request, &CancelFlag::new()).await.unwrap_err().code, ErrorCode::Validation);

    for (url, folder) in [("", None), ("https://github.com/acme/shop.git", Some("bad:name")), ("https://github.com/acme/shop.git", Some("CON"))] {
        let err = s.run(clone_request(url, folder), &CancelFlag::new()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Validation, "{url} {folder:?}");
    }
    assert_eq!(s.runner.calls.lock().unwrap().len(), 1, "only the valid clone ran");
}

#[tokio::test]
async fn laravel_installs_the_installer_when_missing_then_runs_it() {
    let s = Setup::new(World::with_basics());
    s.runner.on(ScriptedCommand { matches: "require laravel/installer".into(), exit: Some(0), creates_files: vec![SCRIPT_84.into()], ..Default::default() });
    s.runner.on(ScriptedCommand {
        matches: " new shop ".into(),
        exit: Some(0),
        creates_dirs: vec!["C:\\sites\\shop".into()],
        lines: vec!["   INFO  Application ready".into()],
        ..Default::default()
    });
    let result = s.run(laravel_request("shop", None), &CancelFlag::new()).await.unwrap();
    assert_eq!(result.folder, "C:\\sites\\shop");
    assert_eq!(result.php_version.as_deref(), Some("8.4"), "newest installed PHP by default");

    let lines = s.runner.command_lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("C:\\lampo\\packages\\php-8.4\\php.exe C:\\lampo\\packages\\composer-2.8\\composer.phar require laravel/installer --no-interaction"), "{}", lines[0]);
    assert_eq!(lines[1], format!("C:\\lampo\\packages\\php-8.4\\php.exe {SCRIPT_84} new shop --no-interaction --vue --pest --no-node --no-boost"));

    let calls = s.runner.calls.lock().unwrap().clone();
    assert_eq!(calls[0].working_dir.as_deref(), Some(std::path::Path::new("C:\\data\\tools\\laravel-installer\\8.4")));
    assert_eq!(calls[1].working_dir.as_deref(), Some(std::path::Path::new("C:\\sites")));
    assert_eq!(calls[1].path_prepend[..2], [PathBuf::from("C:\\data\\tools\\composer\\8.4"), PathBuf::from("C:\\lampo\\packages\\php-8.4")]);
    assert!(calls[1].path_prepend.contains(&PathBuf::from("C:\\Program Files\\Git\\cmd")), "git on PATH when found");
    assert!(calls[1].env.contains(&("PHPRC".into(), "C:\\data\\generated\\php\\_tools-8.4".into())));

    let bat = s.files.contents("C:\\data\\tools\\composer\\8.4\\composer.bat").expect("composer.bat written");
    assert!(bat.contains("\"C:\\lampo\\packages\\php-8.4\\php.exe\" \"C:\\lampo\\packages\\composer-2.8\\composer.phar\" %*"), "{bat}");
    let ini = s.world.ini.0.lock().unwrap().iter().find(|i| i.output_dir.as_path() == std::path::Path::new("C:\\data\\generated\\php\\_tools-8.4")).cloned().expect("tool php.ini");
    assert!(ini.extensions.iter().any(|e| e.name.as_str() == "zip") && ini.extensions.iter().any(|e| e.name.as_str() == "openssl"));
    assert!(s.messages().contains(&"   INFO  Application ready".to_string()));
}

#[tokio::test]
async fn installer_update_failure_is_not_fatal() {
    let s = Setup::new(World::with_basics());
    s.files.file(SCRIPT_84, "");
    s.runner.on(ScriptedCommand { matches: "composer.phar update".into(), exit: Some(1), lines: vec!["curl error 6: could not resolve host".into()], ..Default::default() });
    s.runner.on(ScriptedCommand { matches: " new shop ".into(), exit: Some(0), creates_dirs: vec!["C:\\sites\\shop".into()], ..Default::default() });
    s.run(laravel_request("shop", Some("8.4")), &CancelFlag::new()).await.unwrap();
    let lines = s.runner.command_lines();
    assert!(lines[0].contains("composer.phar update"), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("require")), "already installed: {lines:?}");
    assert!(s.messages().iter().any(|m| m.starts_with("Could not update the Laravel Installer")), "{:?}", s.messages());
}

#[tokio::test]
async fn installer_failure_fails_and_removes_the_partial_folder() {
    let s = Setup::new(World::with_basics());
    s.files.file(SCRIPT_84, "");
    s.runner.on(ScriptedCommand {
        matches: " new shop ".into(),
        exit: Some(1),
        creates_dirs: vec!["C:\\sites\\shop".into()],
        lines: vec!["Your requirements could not be resolved".into()],
        ..Default::default()
    });
    let err = s.run(laravel_request("shop", None), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ProcessFailed);
    assert!(err.message.contains("exit code 1") && err.message.contains("could not be resolved"), "{}", err.message);
    assert_eq!(s.files.path_state(std::path::Path::new("C:\\sites\\shop")), PathState::Missing);
}

#[tokio::test]
async fn a_failed_step_sends_its_last_output_lines_as_one_string() {
    let s = Setup::new(World::with_basics());
    s.runner.on(ScriptedCommand { matches: " clone ".into(), exit: Some(128), lines: vec!["Cloning into 'shop'...".into(), "fatal: repository 'x' not found".into()], ..Default::default() });
    let err = s.run(clone_request("https://github.com/acme/shop.git", None), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ProcessFailed);
    let details = err.details.expect("details");
    assert_eq!(details["output"], "Cloning into 'shop'...\nfatal: repository 'x' not found", "the same type as `config-rejected`: {details}");
}

#[tokio::test]
async fn cancellation_removes_only_a_folder_this_run_created() {
    let s = Setup::new(World::with_basics());
    s.runner.on(ScriptedCommand {
        matches: " clone ".into(),
        creates_dirs: vec!["C:\\sites\\shop".into(), "C:\\sites\\keep".into()],
        creates_files: vec!["C:\\sites\\shop\\.git\\HEAD".into(), "C:\\sites\\keep\\.git\\HEAD".into()],
        until_cancelled: true,
        ..Default::default()
    });
    let cancel = CancelFlag::new();
    cancel.cancel();
    let err = s.run(clone_request("https://github.com/acme/shop.git", None), &cancel).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert_eq!(s.files.path_state(std::path::Path::new("C:\\sites\\shop")), PathState::Missing, "created by this run: removed");
    assert_eq!(*s.files.removed.lock().unwrap(), ["c:\\sites\\shop"]);

    // A folder that existed (empty) before the run is left in place.
    let s = Setup::new(World::with_basics());
    s.files.dir("C:\\sites\\keep");
    s.runner.on(ScriptedCommand { matches: " clone ".into(), creates_files: vec!["C:\\sites\\keep\\.git\\HEAD".into()], until_cancelled: true, ..Default::default() });
    let err = s.run(clone_request("https://github.com/acme/shop.git", Some("keep")), &cancel).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert!(s.files.removed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn laravel_needs_php_8_2_and_composer() {
    // No PHP at all.
    let s = Setup::new(World::new());
    let err = s.run(laravel_request("shop", None), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PackageMissing);

    // The chosen PHP is not installed.
    let s = Setup::new(World::with_basics());
    let err = s.run(laravel_request("shop", Some("8.2")), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PackageMissing);
    assert!(err.message.contains("PHP 8.2"));

    // Too old for the installer.
    let w = World::with_basics();
    w.installed.add(PackageKind::PHP, "8.1.30", None);
    let s = Setup::new(w);
    let err = s.run(laravel_request("shop", Some("8.1")), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);

    // No Composer.
    let w = World::new();
    w.installed.add(PackageKind::PHP, "8.4.1", None);
    let s = Setup::new(w);
    let err = s.run(laravel_request("shop", None), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PackageMissing);
    assert!(err.message.contains("Composer"));

    // `--git` needs git; invalid names are rejected before anything runs.
    let s = Setup::new(World::with_basics());
    s.locator.0.lock().unwrap().clear();
    let mut request = laravel_request("shop", None);
    if let ScaffoldSourceDto::Laravel { git, .. } = &mut request.source {
        *git = true;
    }
    assert_eq!(s.run(request, &CancelFlag::new()).await.unwrap_err().details.unwrap()["tool"], "git");
    assert_eq!(s.run(laravel_request("my shop", None), &CancelFlag::new()).await.unwrap_err().code, ErrorCode::Validation);
    assert!(s.runner.calls.lock().unwrap().is_empty());
}

#[test]
fn request_wire_format() {
    let json = serde_json::json!({
        "parentDir": "C:\\sites",
        "folderName": "shop",
        "source": { "kind": "laravel", "phpVersion": "8.4", "starterKit": "svelte", "testing": "phpunit", "git": true, "npm": true }
    });
    let r: ScaffoldProjectRequest = serde_json::from_value(json).unwrap();
    assert_eq!(r.source, ScaffoldSourceDto::Laravel { php_version: Some("8.4".into()), starter_kit: StarterKitDto::Svelte, testing: TestingFrameworkDto::Phpunit, git: true, npm: true });
    let git: ScaffoldProjectRequest = serde_json::from_value(serde_json::json!({ "parentDir": "C:\\sites", "source": { "kind": "git-clone", "url": "u" } })).unwrap();
    assert_eq!(git.source, ScaffoldSourceDto::GitClone { url: "u".into(), branch: None });
    let op = serde_json::to_value(TaskOperationDto::ScaffoldProject { request: git }).unwrap();
    assert_eq!(op["kind"], "scaffold-project");
}
