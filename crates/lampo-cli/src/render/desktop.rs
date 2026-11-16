//! `open`: what was started, on which folder or URL.

use lampo_application::dto::{ExternalApp, OpenedInDto};

/// The app as the user knows it.
fn app_name(app: ExternalApp) -> &'static str {
    match app {
        ExternalApp::Terminal => "a PowerShell console",
        ExternalApp::Folder => "Explorer",
        ExternalApp::VsCode => "Visual Studio Code",
        ExternalApp::Browser => "the browser",
    }
}

pub fn opened(o: &OpenedInDto) -> String {
    format!("{}: opened {} in {}", o.site, o.target, app_name(o.app))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_site_the_target_and_the_app() {
        let dto = |app, target: &str| OpenedInDto { site: "shop@fix".into(), app, target: target.into(), served: true };
        assert_eq!(opened(&dto(ExternalApp::VsCode, r"C:\work\shop-fix")), r"shop@fix: opened C:\work\shop-fix in Visual Studio Code");
        assert_eq!(opened(&dto(ExternalApp::Browser, "http://127.0.0.1:8150")), "shop@fix: opened http://127.0.0.1:8150 in the browser");
        assert_eq!(opened(&dto(ExternalApp::Terminal, r"C:\w")), r"shop@fix: opened C:\w in a PowerShell console");
        assert_eq!(opened(&dto(ExternalApp::Folder, r"C:\w")), r"shop@fix: opened C:\w in Explorer");
    }
}
