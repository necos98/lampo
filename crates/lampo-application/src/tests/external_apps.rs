//! Opening a project folder in the user's desktop apps.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lampo_domain::ports::ExternalAppLauncher;
use lampo_domain::project::ProjectId;
use lampo_domain::{PortError, PortResult};

use crate::error::ErrorCode;
use crate::testing::*;
use crate::use_cases::*;

const ROOT: &str = r"C:\sites\shop";

/// Records every launch and answers with `failure` when one is set.
#[derive(Default)]
struct FakeExternalApps {
    launches: Mutex<Vec<(ExternalApp, Option<String>, PathBuf)>>,
    failure: Mutex<Option<PortError>>,
}

impl FakeExternalApps {
    fn failing(error: PortError) -> Self {
        Self { failure: Mutex::new(Some(error)), ..Default::default() }
    }

    fn record(&self, app: ExternalApp, project: Option<&ProjectId>, folder: &Path) -> PortResult<()> {
        self.launches.lock().unwrap().push((app, project.map(|p| p.to_string()), folder.to_path_buf()));
        self.failure.lock().unwrap().clone().map_or(Ok(()), Err)
    }

    fn launches(&self) -> Vec<(ExternalApp, Option<String>, PathBuf)> {
        self.launches.lock().unwrap().clone()
    }
}

impl ExternalAppLauncher for FakeExternalApps {
    fn open_folder(&self, folder: &Path) -> PortResult<()> {
        self.record(ExternalApp::Folder, None, folder)
    }
    fn open_file(&self, file: &Path) -> PortResult<()> {
        unreachable!("a project is never opened as a file ({})", file.display())
    }
    fn open_in_vscode(&self, folder: &Path) -> PortResult<()> {
        self.record(ExternalApp::VsCode, None, folder)
    }
    fn open_terminal(&self, project: &ProjectId, folder: &Path) -> PortResult<()> {
        self.record(ExternalApp::Terminal, Some(project), folder)
    }
}

fn open(w: &World, launcher: &Arc<FakeExternalApps>) -> OpenProjectIn {
    OpenProjectIn::new(w.projects.clone(), w.fs.clone(), launcher.clone())
}

#[test]
fn opens_the_project_folder_in_each_app() {
    let w = World::new();
    w.project("shop", ROOT, true);
    let launcher = Arc::new(FakeExternalApps::default());
    let open = open(&w, &launcher);

    for app in [ExternalApp::Terminal, ExternalApp::Folder, ExternalApp::VsCode] {
        open.execute("shop", app).unwrap();
    }

    let root = PathBuf::from(ROOT);
    assert_eq!(launcher.launches(), vec![(ExternalApp::Terminal, Some("shop".to_string()), root.clone()), (ExternalApp::Folder, None, root.clone()), (ExternalApp::VsCode, None, root)]);
}

#[test]
fn unknown_project_is_not_found_and_launches_nothing() {
    let w = World::new();
    let launcher = Arc::new(FakeExternalApps::default());

    let err = open(&w, &launcher).execute("nope", ExternalApp::Folder).unwrap_err();

    assert_eq!(err.code, ErrorCode::NotFound);
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
fn the_terminal_is_launched_even_when_the_folder_is_missing() {
    let w = World::new();
    w.project("shop", ROOT, true);
    w.fs.remove(ROOT);
    let launcher = Arc::new(FakeExternalApps::default());

    open(&w, &launcher).execute("shop", ExternalApp::Terminal).unwrap();

    assert_eq!(launcher.launches().len(), 1);
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
fn a_folder_vscode_cannot_open_is_a_validation_error_with_the_launcher_reason() {
    let w = World::new();
    w.project("shop", ROOT, true);
    let launcher = Arc::new(FakeExternalApps::failing(PortError::Unsupported("rename the folder".into())));

    let err = open(&w, &launcher).execute("shop", ExternalApp::VsCode).unwrap_err();

    assert_eq!(err.code, ErrorCode::Validation);
    assert_eq!(err.message, "rename the folder");
}

#[test]
fn a_failed_start_names_what_could_not_be_opened() {
    let w = World::new();
    w.project("shop", ROOT, true);
    let cases = [
        (ExternalApp::Terminal, "could not open the terminal: access denied"),
        (ExternalApp::Folder, "could not open the folder: access denied"),
        (ExternalApp::VsCode, "could not start Visual Studio Code: access denied"),
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
