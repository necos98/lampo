//! DTOs of opening a site in the user's desktop apps (the GUI's buttons, `lampo open`).

use serde::{Deserialize, Serialize};

/// The desktop app to open a site in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalApp {
    /// A PowerShell console with the site's toolchain on PATH.
    Terminal,
    /// The file manager (Windows Explorer).
    #[serde(rename = "explorer")]
    Folder,
    /// Visual Studio Code.
    #[serde(rename = "code")]
    VsCode,
    /// The default web browser, on the site's primary URL.
    Browser,
}

/// What was started: the app, and the folder or URL it was given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedInDto {
    /// `shop` or `shop@feature-x`.
    pub site: String,
    /// `terminal`, `explorer`, `code` or `browser`.
    pub app: ExternalApp,
    /// The site folder (terminal, explorer, code) or its primary URL (browser).
    pub target: String,
    /// `false` when Apache does not serve the site (an inactive project): the browser does not
    /// reach it until the project is activated.
    pub served: bool,
}
