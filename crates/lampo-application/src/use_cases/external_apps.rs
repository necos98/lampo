//! Opening a project folder in the user's desktop apps (terminal, Explorer, VS Code).

use std::sync::Arc;

use lampo_domain::PortError;
use lampo_domain::ports::{ExternalAppLauncher, FileSystemInspector, ProjectRepository};
use lampo_domain::project::ProjectRoot;

use super::support::load_project;
use crate::error::{AppError, AppResult, ErrorCode};

const VSCODE_NOT_FOUND: &str = "Visual Studio Code non trovato: installalo o aggiungi `code` al PATH";

/// The desktop app to open a project folder in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalApp {
    /// A PowerShell console with the project's toolchain on PATH.
    Terminal,
    /// The file manager (Windows Explorer).
    Folder,
    /// Visual Studio Code.
    VsCode,
}

impl ExternalApp {
    /// What failed, as in "impossibile {action}: {reason}".
    fn action(self) -> &'static str {
        match self {
            Self::Terminal => "aprire il terminale",
            Self::Folder => "aprire la cartella",
            Self::VsCode => "avviare Visual Studio Code",
        }
    }
}

/// Opens a project folder in an [`ExternalApp`]. Starts a process of the user, so it runs where
/// the user is (the GUI, or the CLI on the user's machine); it changes nothing in Lampo.
pub struct OpenProjectIn {
    projects: Arc<dyn ProjectRepository>,
    fs: Arc<dyn FileSystemInspector>,
    launcher: Arc<dyn ExternalAppLauncher>,
}

impl OpenProjectIn {
    pub fn new(projects: Arc<dyn ProjectRepository>, fs: Arc<dyn FileSystemInspector>, launcher: Arc<dyn ExternalAppLauncher>) -> Self {
        Self { projects, fs, launcher }
    }

    /// `NotFound` for an unknown project; `Validation` when its folder is missing (Explorer,
    /// VS Code) or VS Code cannot open it; `PackageMissing` when VS Code is not installed.
    pub fn execute(&self, project_id: &str, app: ExternalApp) -> AppResult<()> {
        let project = load_project(self.projects.as_ref(), project_id)?;
        let folder = project.root.as_path();
        let launched = match app {
            ExternalApp::Terminal => self.launcher.open_terminal(&project.id, folder),
            ExternalApp::Folder => {
                self.ensure_folder(&project.root)?;
                self.launcher.open_folder(folder)
            }
            ExternalApp::VsCode => {
                self.ensure_folder(&project.root)?;
                self.launcher.open_in_vscode(folder)
            }
        };
        launched.map_err(|e| launch_error(app, e))
    }

    fn ensure_folder(&self, root: &ProjectRoot) -> AppResult<()> {
        if self.fs.is_dir(root.as_path()) { Ok(()) } else { Err(AppError::new(ErrorCode::Validation, format!("la cartella del progetto non esiste: {root}"))) }
    }
}

/// The error a front-end shows when `app` could not be started.
fn launch_error(app: ExternalApp, error: PortError) -> AppError {
    match error {
        PortError::NotFound { .. } if app == ExternalApp::VsCode => AppError::new(ErrorCode::PackageMissing, VSCODE_NOT_FOUND),
        PortError::Unsupported(reason) => AppError::new(ErrorCode::Validation, reason),
        PortError::Process { message, .. } => AppError::internal(format!("impossibile {}: {message}", app.action())),
        PortError::Io { message, .. } => AppError::internal(message),
        other => other.into(),
    }
}
