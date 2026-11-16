//! Opening a site (project or worktree environment) in the user's desktop apps.

use std::path::Path;
use std::sync::{Arc, Mutex};

use lampo_domain::ports::{ExternalAppLauncher, ProjectRepository, SettingsRepository, WorktreeRepository};
use lampo_domain::project::{ProjectId, ProjectRoot};
use lampo_domain::site::SiteId;
use lampo_domain::tcp_port::Port;
use lampo_domain::worktree::{DatabaseMode, Occupancy, WorktreeEnvironment};
use lampo_domain::{PortError, PortResult};

use crate::dto::OpenedInDto;
use crate::error::ErrorCode;
use crate::testing::*;
use crate::use_cases::*;

const ROOT: &str = r"C:\sites\shop";
const WORKTREE_ROOT: &str = r"C:\sites\shop-fix";
const WORKTREE_PORT: u16 = 8150;

/// One launch: the app, the site of a terminal, and the folder or URL it was given.
type Launch = (ExternalApp, Option<String>, String);

/// Records every launch and answers with `failure` when one is set.
#[derive(Default)]
struct FakeExternalApps {
    launches: Mutex<Vec<Launch>>,
    failure: Mutex<Option<PortError>>,
}

impl FakeExternalApps {
    fn failing(error: PortError) -> Self {
        Self { failure: Mutex::new(Some(error)), ..Default::default() }
    }

    fn record(&self, app: ExternalApp, site: Option<&SiteId>, target: impl Into<String>) -> PortResult<()> {
        self.launches.lock().unwrap().push((app, site.map(|s| s.to_string()), target.into()));
        self.failure.lock().unwrap().clone().map_or(Ok(()), Err)
    }

    fn launches(&self) -> Vec<Launch> {
        self.launches.lock().unwrap().clone()
    }
}

impl ExternalAppLauncher for FakeExternalApps {
    fn open_folder(&self, folder: &Path) -> PortResult<()> {
        self.record(ExternalApp::Folder, None, folder.display().to_string())
    }
    fn open_file(&self, file: &Path) -> PortResult<()> {
        unreachable!("a site is never opened as a file ({})", file.display())
    }
    fn open_in_vscode(&self, folder: &Path) -> PortResult<()> {
        self.record(ExternalApp::VsCode, None, folder.display().to_string())
    }
    fn open_terminal(&self, site: &SiteId, folder: &Path) -> PortResult<()> {
        self.record(ExternalApp::Terminal, Some(site), folder.display().to_string())
    }
    fn open_url(&self, url: &str) -> PortResult<()> {
        self.record(ExternalApp::Browser, None, url)
    }
}

fn open(w: &World, launcher: &Arc<FakeExternalApps>) -> OpenProjectIn {
    OpenProjectIn::new(OpenProjectInDeps { projects: w.projects.clone(), worktrees: w.worktrees.clone(), settings: w.settings.clone(), fs: w.fs.clone(), launcher: launcher.clone() })
}

/// A worktree environment of `shop` in [`WORKTREE_ROOT`], served on [`WORKTREE_PORT`].
fn worktree(w: &World) -> WorktreeEnvironment {
    let parent = w.projects.get(&ProjectId::new("shop").unwrap()).unwrap().unwrap();
    let env =
        WorktreeEnvironment::derive(&parent, &Occupancy::default(), ProjectRoot::new(WORKTREE_ROOT).unwrap(), Some("fix".into()), Port::of(WORKTREE_PORT), &DatabaseMode::Shared, 1).unwrap();
    w.worktrees.save(&env).unwrap();
    w.fs.add(WORKTREE_ROOT);
    env
}

#[test]
fn opens_the_project_folder_in_each_app() {
    let w = World::new();
    w.project("shop", ROOT, true);
    let launcher = Arc::new(FakeExternalApps::default());
    let open = open(&w, &launcher);

    for app in [ExternalApp::Terminal, ExternalApp::Folder, ExternalApp::VsCode] {
        let opened = open.execute("shop", app).unwrap();
        assert_eq!(opened, OpenedInDto { site: "shop".into(), app, target: ROOT.into(), served: true });
    }

    assert_eq!(
        launcher.launches(),
        vec![(ExternalApp::Terminal, Some("shop".to_string()), ROOT.to_string()), (ExternalApp::Folder, None, ROOT.to_string()), (ExternalApp::VsCode, None, ROOT.to_string())]
    );
}

#[test]
fn the_browser_opens_the_primary_url_of_the_project() {
    let w = World::new();
    let project = w.project("shop", ROOT, false);
    let url = project.url(&w.settings.load().unwrap());
    let launcher = Arc::new(FakeExternalApps::default());

    let opened = open(&w, &launcher).execute("shop", ExternalApp::Browser).unwrap();

    assert_eq!(opened, OpenedInDto { site: "shop".into(), app: ExternalApp::Browser, target: url.clone(), served: false }, "an inactive project is not served");
    assert_eq!(launcher.launches(), vec![(ExternalApp::Browser, None, url)]);
}

#[test]
fn a_worktree_environment_opens_its_own_folder_url_and_toolchain() {
    let w = World::new();
    w.project("shop", ROOT, false);
    let env = worktree(&w);
    let site = env.site_id().to_string();
    let launcher = Arc::new(FakeExternalApps::default());
    let open = open(&w, &launcher);

    for app in [ExternalApp::Terminal, ExternalApp::Folder, ExternalApp::VsCode, ExternalApp::Browser] {
        assert!(open.execute(&site, app).unwrap().served, "worktree environments are always served");
    }

    let url = format!("http://127.0.0.1:{WORKTREE_PORT}");
    assert_eq!(
        launcher.launches(),
        vec![
            (ExternalApp::Terminal, Some(site.clone()), WORKTREE_ROOT.to_string()),
            (ExternalApp::Folder, None, WORKTREE_ROOT.to_string()),
            (ExternalApp::VsCode, None, WORKTREE_ROOT.to_string()),
            (ExternalApp::Browser, None, url),
        ]
    );
}

#[test]
fn unknown_sites_are_not_found_and_launch_nothing() {
    let w = World::new();
    w.project("shop", ROOT, true);
    let launcher = Arc::new(FakeExternalApps::default());
    let open = open(&w, &launcher);

    for site in ["nope", "shop@nope"] {
        let err = open.execute(site, ExternalApp::Folder).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound, "{site}");
    }
    assert!(launcher.launches().is_empty());
}

#[test]
fn a_missing_folder_is_refused_before_explorer_and_vscode() {
    let w = World::new();
    w.project("shop", ROOT, true);
    w.fs.remove(ROOT);
    let launcher = Arc::new(FakeExternalApps::default());
    let open = open(&w, &launcher);

    for app in [ExternalApp::Folder, ExternalApp::VsCode] {
        let err = open.execute("shop", app).unwrap_err();
        assert_eq!(err.code, ErrorCode::Validation);
        assert_eq!(err.message, format!("the project folder does not exist: {ROOT}"));
        assert_eq!(err.details.unwrap()["folder"], ROOT, "the GUI names the folder");
    }
    assert!(launcher.launches().is_empty());
}

#[test]
fn the_terminal_and_the_browser_are_launched_even_when_the_folder_is_missing() {
    let w = World::new();
    w.project("shop", ROOT, true);
    w.fs.remove(ROOT);
    let launcher = Arc::new(FakeExternalApps::default());
    let open = open(&w, &launcher);

    open.execute("shop", ExternalApp::Terminal).unwrap();
    open.execute("shop", ExternalApp::Browser).unwrap();

    assert_eq!(launcher.launches().len(), 2);
}

#[test]
fn vscode_not_installed_is_package_missing() {
    let w = World::new();
    w.project("shop", ROOT, true);
    let launcher = Arc::new(FakeExternalApps::failing(PortError::not_found("Visual Studio Code", "code.cmd")));

    let err = open(&w, &launcher).execute("shop", ExternalApp::VsCode).unwrap_err();

    assert_eq!(err.code, ErrorCode::PackageMissing);
    assert_eq!(err.message, "Visual Studio Code not found: install it or add `code` to PATH");
    assert_eq!(err.details.unwrap()["tool"], "vscode", "the GUI recognises VS Code");
}

#[test]
fn a_target_the_launcher_refuses_is_a_validation_error_with_its_reason() {
    let w = World::new();
    w.project("shop", ROOT, true);

    for app in [ExternalApp::VsCode, ExternalApp::Browser] {
        let launcher = Arc::new(FakeExternalApps::failing(PortError::Unsupported("rename the folder".into())));
        let err = open(&w, &launcher).execute("shop", app).unwrap_err();
        assert_eq!(err.code, ErrorCode::Validation, "{app:?}");
        assert_eq!(err.message, "rename the folder");
    }
}

#[test]
fn a_failed_start_names_what_could_not_be_opened() {
    let w = World::new();
    w.project("shop", ROOT, true);
    let cases = [
        (ExternalApp::Terminal, "could not open the terminal: access denied"),
        (ExternalApp::Folder, "could not open the folder: access denied"),
        (ExternalApp::VsCode, "could not start Visual Studio Code: access denied"),
        (ExternalApp::Browser, "could not open the browser: access denied"),
    ];

    for (app, expected) in cases {
        let launcher = Arc::new(FakeExternalApps::failing(PortError::process("program.exe", "access denied")));
        let err = open(&w, &launcher).execute("shop", app).unwrap_err();
        assert_eq!(err.code, ErrorCode::ProcessFailed, "{app:?}");
        assert_eq!(err.message, expected);
    }
}

#[test]
fn a_cli_that_cannot_be_located_reports_the_os_error() {
    let w = World::new();
    w.project("shop", ROOT, true);
    let launcher = Arc::new(FakeExternalApps::failing(PortError::io("locating the lampo CLI", "file not found")));

    let err = open(&w, &launcher).execute("shop", ExternalApp::Terminal).unwrap_err();

    assert_eq!(err.code, ErrorCode::Internal);
    assert_eq!(err.message, "file not found");
}

#[test]
fn the_app_names_are_stable_on_the_wire() {
    let names: Vec<_> = [ExternalApp::Terminal, ExternalApp::Folder, ExternalApp::VsCode, ExternalApp::Browser].iter().map(|a| serde_json::to_value(a).unwrap()).collect();
    assert_eq!(names, ["terminal", "explorer", "code", "browser"]);
}
