//! `package ...`.

use lampo_application::control::ControlRequest;
use lampo_application::dto::*;
use lampo_application::AppResult;
use lampo_domain::package::PackageKind;
use serde::Serialize;

use super::{Cmd, validation};
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
            let defender_exclusion = defender_suggestions(c, &installed.id).await;
            let change = PackageChange { package: installed, defender_exclusion };
            c.io.emit(&change, |change| render::with_defender_suggestions(render::installed_one(&change.package), &change.defender_exclusion));
        }
        PackageCommand::Uninstall { kind, package_version, yes: false, php } => {
            let what = [Some(kind), package_version, php.map(|p| format!("--php {p}"))].into_iter().flatten().collect::<Vec<_>>().join(" ");
            return Err(validation(format!("refusing to uninstall `{what}` without --yes (lampo never prompts)")));
        }
        PackageCommand::Uninstall { kind, package_version, php, yes: true } => {
            let (version, series) = split_version(package_version.as_deref());
            let request = UninstallPackageRequest { kind, version, series, php_version: php };
            let removed: PackageIdDto = c.app(ControlRequest::PackageUninstall(request)).await?;
            let defender_exclusion = defender_suggestions(c, &removed).await;
            let change = PackageChange { package: removed, defender_exclusion };
            c.io.emit(&change, |change| render::with_defender_suggestions(render::uninstalled(&change.package), &change.defender_exclusion));
        }
        PackageCommand::Extensions { php } => {
            let list = c.ctx.php_extensions.list(&c.resolve_request(), php.as_deref())?;
            c.io.emit(&list, render::php_extensions);
        }
    }
    Ok(0)
}

/// What `install` and `uninstall` print: the package, and after a PHP the Defender exclusion
/// changes Lampo proposes (`defenderExclusion`, only when there is one). Never made from here:
/// each is a command the user runs.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageChange<T> {
    #[serde(flatten)]
    package: T,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    defender_exclusion: Vec<DefenderExclusionSuggestionDto>,
}

/// The changes to propose after `id` was installed or uninstalled: only for a PHP, so Defender's
/// state (a few seconds at most) is never read for another package.
async fn defender_suggestions(c: &Cmd<'_>, id: &PackageIdDto) -> Vec<DefenderExclusionSuggestionDto> {
    match id.kind == PackageKind::PHP.as_str() {
        true => c.ctx.defender_advice.suggestions().await,
        false => Vec::new(),
    }
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
