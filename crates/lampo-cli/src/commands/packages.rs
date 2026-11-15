//! `package ...`.

use lampo_application::control::ControlRequest;
use lampo_application::dto::*;
use lampo_application::AppResult;

use super::Cmd;
use crate::cli::PackageCommand;
use crate::render;

pub async fn run(c: &Cmd<'_>, command: PackageCommand) -> AppResult<i32> {
    match command {
        PackageCommand::List { kind, installed: true } => {
            let mut list = c.ctx.packages.installed()?;
            if let Some(kind) = &kind {
                list.retain(|p| &p.id.kind == kind);
            }
            c.io.emit(&list, |l| render::installed(l));
        }
        PackageCommand::List { kind, installed: false } => {
            let list = c.ctx.packages.catalog(kind.as_deref()).await?;
            c.io.emit(&list, |l| render::catalog(l));
        }
        PackageCommand::Install { kind, package_version, php } => {
            let (version, series) = split_version(package_version.as_deref());
            let request = InstallPackageRequest { kind, version, series, php_version: php };
            let installed: InstalledPackageDto = c.app(ControlRequest::PackageInstall(request)).await?;
            c.io.emit(&installed, render::installed_one);
        }
    }
    Ok(0)
}

/// `8.3` (up to two numeric parts) is a series: latest patch. `8.3.12` (three or more) is an
/// exact version. Returns `(version, series)`.
pub fn split_version(value: Option<&str>) -> (Option<String>, Option<String>) {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        None => (None, None),
        Some(v) if v.split('.').count() >= 3 => (Some(v.to_string()), None),
        Some(v) => (None, Some(v.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::split_version;

    #[test]
    fn series_versus_exact_version() {
        assert_eq!(split_version(None), (None, None));
        assert_eq!(split_version(Some("8.3")), (None, Some("8.3".into())));
        assert_eq!(split_version(Some("8")), (None, Some("8".into())));
        assert_eq!(split_version(Some("8.3.12")), (Some("8.3.12".into()), None));
        assert_eq!(split_version(Some(" ")), (None, None));
    }
}
