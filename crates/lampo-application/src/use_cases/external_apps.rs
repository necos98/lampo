//! Opening a site (a project or a worktree environment) in the user's desktop apps: terminal,
//! Explorer, VS Code, web browser.

use std::sync::Arc;

use lampo_domain::PortError;
use lampo_domain::ports::{ExternalAppLauncher, FileSystemInspector, ProjectRepository, SettingsRepository, WorktreeRepository};
use lampo_domain::project::ProjectRoot;

use super::support::{SiteRef, load_site};
pub use crate::dto::ExternalApp;
use crate::dto::OpenedInDto;
use crate::error::{AppError, AppResult, ErrorCode};

const VSCODE_NOT_FOUND: &str = "Visual Studio Code not found: install it or add `code` to PATH";
/// `details.tool` of the missing-VS Code error, recognisable by the GUI.
const VSCODE_TOOL: &str = "vscode";

impl ExternalApp {
    /// What failed, as in "could not {action}: {reason}".
    fn action(self) -> &'static str {
        match self {
            Self::Terminal => "open the terminal",
            Self::Folder => "open the folder",
            Self::VsCode => "start Visual Studio Code",
            Self::Browser => "open the browser",
        }
    }
}

/// The ports [`OpenProjectIn`] reads and starts apps through.
pub struct OpenProjectInDeps {
    pub projects: Arc<dyn ProjectRepository>,
    pub worktrees: Arc<dyn WorktreeRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub fs: Arc<dyn FileSystemInspector>,
    pub launcher: Arc<dyn ExternalAppLauncher>,
}

/// Opens a site in an [`ExternalApp`]. Starts a process of the user, so it runs where the user
/// is (the GUI, or the CLI on the user's machine); it changes nothing in Lampo.
pub struct OpenProjectIn {
    deps: OpenProjectInDeps,
}

impl OpenProjectIn {
    pub fn new(deps: OpenProjectInDeps) -> Self {
        Self { deps }
    }

    /// `site_id` is a project (`shop`) or a worktree environment (`shop@feature-x`), opened on
    /// its own folder and URL.
    ///
    /// `NotFound` for an unknown site; `Validation` when its folder is missing (Explorer,
    /// VS Code; `details.folder`) or VS Code cannot open it; `PackageMissing` when VS Code is not
    /// installed (`details.tool`); `ProcessFailed` when the app cannot be started.
    pub fn execute(&self, site_id: &str, app: ExternalApp) -> AppResult<OpenedInDto> {
        let (id, project, worktree) = load_site(self.deps.projects.as_ref(), self.deps.worktrees.as_ref(), site_id)?;
        let site = SiteRef { id, project: &project, worktree: worktree.as_ref() };
        let root = site.root();
        let launcher = self.deps.launcher.as_ref();
        let (target, launched) = match app {
            ExternalApp::Terminal => (root.to_string(), launcher.open_terminal(&site.id, root.as_path())),
            ExternalApp::Folder => {
                self.ensure_folder(root)?;
                (root.to_string(), launcher.open_folder(root.as_path()))
            }
            ExternalApp::VsCode => {
                self.ensure_folder(root)?;
                (root.to_string(), launcher.open_in_vscode(root.as_path()))
            }
            ExternalApp::Browser => {
                let url = self.url(&site)?;
                let launched = launcher.open_url(&url);
                (url, launched)
            }
        };
        launched.map_err(|e| launch_error(app, e))?;
        Ok(OpenedInDto { site: site.id.to_string(), app, target, served: site.served() })
    }

    /// The site's primary URL: the loopback one of a worktree environment, the project's own.
    fn url(&self, site: &SiteRef<'_>) -> AppResult<String> {
        Ok(match site.worktree {
            Some(w) => w.url(),
            None => site.project.url(&self.deps.settings.load()?),
        })
    }

    fn ensure_folder(&self, root: &ProjectRoot) -> AppResult<()> {
        if self.deps.fs.is_dir(root.as_path()) {
            return Ok(());
        }
        Err(AppError::new(ErrorCode::Validation, format!("the project folder does not exist: {root}")).with_details(serde_json::json!({ "folder": root.to_string() })))
    }
}

/// The error a front-end shows when `app` could not be started.
fn launch_error(app: ExternalApp, error: PortError) -> AppError {
    match error {
        PortError::NotFound { .. } if app == ExternalApp::VsCode => AppError::new(ErrorCode::PackageMissing, VSCODE_NOT_FOUND).with_details(serde_json::json!({ "tool": VSCODE_TOOL })),
        PortError::Unsupported(reason) => AppError::new(ErrorCode::Validation, reason),
        PortError::Process { message, .. } => AppError::new(ErrorCode::ProcessFailed, format!("could not {}: {message}", app.action())),
        PortError::Io { message, .. } => AppError::internal(message),
        other => other.into(),
    }
}
