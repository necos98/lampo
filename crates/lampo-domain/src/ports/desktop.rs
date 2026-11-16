//! The user's own desktop apps (terminal, file manager, code editor), opened on a project folder
//! or on a file.

use std::path::Path;

use crate::error::PortResult;
use crate::project::ProjectId;

/// Starts an external desktop app on a folder and returns without waiting for it: the app is an
/// ordinary process of the user, never supervised nor stopped by Lampo. Implemented by
/// `lampo-infra-services` (`WindowsExternalAppLauncher`).
///
/// Errors: `Process` when the app cannot be started (`message` is the OS error).
pub trait ExternalAppLauncher: Send + Sync {
    /// Opens `folder` in the file manager (Windows Explorer).
    fn open_folder(&self, folder: &Path) -> PortResult<()>;
    /// Opens `file` in the app Windows associates with its type (Notepad for a `.log`).
    fn open_file(&self, file: &Path) -> PortResult<()>;
    /// Opens `folder` in Visual Studio Code. `NotFound` when it is not installed;
    /// `Unsupported(reason)` when the install found cannot open this folder (`reason` is
    /// user-facing).
    fn open_in_vscode(&self, folder: &Path) -> PortResult<()>;
    /// Opens a PowerShell console in `folder` with the toolchain of `project` on PATH
    /// (`lampo -p <project> exec -- powershell`). `Io` when the `lampo` CLI cannot be located.
    fn open_terminal(&self, project: &ProjectId, folder: &Path) -> PortResult<()>;
}
