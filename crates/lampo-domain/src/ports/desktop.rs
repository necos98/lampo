//! The user's own desktop apps (terminal, file manager, code editor, web browser), opened on a
//! site folder, a file or a site URL.

use std::path::Path;

use crate::error::PortResult;
use crate::site::SiteId;

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
    /// Opens a PowerShell console in `folder` with the toolchain of `site` on PATH
    /// (`lampo -p <site> exec -- powershell`). `Io` when the `lampo` CLI cannot be located.
    fn open_terminal(&self, site: &SiteId, folder: &Path) -> PortResult<()>;
    /// Opens `url` in the default web browser. `Unsupported(reason)` for anything but an
    /// `http://` or `https://` URL, so no other protocol handler can be started through it.
    fn open_url(&self, url: &str) -> PortResult<()>;
}
