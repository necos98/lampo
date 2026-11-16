//! Arguments of `open`: the desktop app to open the site in.

use clap::{ArgGroup, Args};
use lampo_application::dto::ExternalApp;

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("app").args(["code", "explorer", "terminal", "browser"])))]
pub struct OpenArgs {
    /// Open the site folder in Visual Studio Code.
    #[arg(long)]
    pub code: bool,
    /// Open the site folder in Windows Explorer (the default).
    #[arg(long)]
    pub explorer: bool,
    /// Open a PowerShell console in the site folder with the site's toolchain on PATH.
    #[arg(long)]
    pub terminal: bool,
    /// Open the site's primary URL in the default web browser.
    #[arg(long)]
    pub browser: bool,
}

impl OpenArgs {
    /// The app the flags name; Explorer when none is given.
    pub fn app(&self) -> ExternalApp {
        if self.code {
            ExternalApp::VsCode
        } else if self.terminal {
            ExternalApp::Terminal
        } else if self.browser {
            ExternalApp::Browser
        } else {
            ExternalApp::Folder
        }
    }
}
