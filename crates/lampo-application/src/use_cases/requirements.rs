//! Which packages the configuration needs, and who needs them. One rule set shared by
//! `doctor` (missing packages), the package views (`used_by`) and uninstall (in-use check).

use lampo_domain::package::{InstalledPackage, PackageId, PackageKind};
use lampo_domain::php::PhpMinor;
use lampo_domain::project::Project;
use lampo_domain::settings::Settings;
use lampo_domain::version::VersionSeries;
use lampo_domain::worktree::WorktreeEnvironment;

use super::service_catalog::ServiceCatalog;
use super::support::{all_sites, composer_series};
use lampo_domain::service::ServiceId;

/// Who needs a package when no site does (always-on MySQL, the default CLI PHP).
pub(crate) const SYSTEM: &str = "system";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Requirement {
    pub kind: PackageKind,
    /// `None` = any series.
    pub series: Option<VersionSeries>,
    /// For PHP add-ons: the PHP minor the build must target.
    pub php: Option<PhpMinor>,
    /// Site id or [`SYSTEM`].
    pub required_by: String,
}

impl Requirement {
    fn new(kind: PackageKind, series: Option<VersionSeries>, php: Option<PhpMinor>, required_by: impl Into<String>) -> Self {
        Self { kind, series, php, required_by: required_by.into() }
    }

    pub fn matches(&self, id: &PackageId) -> bool {
        id.kind == self.kind && self.series.is_none_or(|s| id.series() == s) && id.php == self.php
    }

    pub fn satisfied_by(&self, installed: &[InstalledPackage]) -> bool {
        installed.iter().any(|p| self.matches(&p.id))
    }

    /// `php 8.2`, `mysql 5.7`, `xdebug for PHP 8.3`, `apache`.
    pub fn describe(&self) -> String {
        let mut s = self.kind.to_string();
        if let Some(series) = self.series {
            s.push_str(&format!(" {series}"));
        }
        if let Some(php) = self.php {
            s.push_str(&format!(" for PHP {php}"));
        }
        s
    }
}

/// Requirements of every site (served or not: shims use inactive projects too) plus the system.
pub(crate) fn requirements(projects: &[Project], worktrees: &[WorktreeEnvironment], settings: &Settings, catalog: &ServiceCatalog) -> Vec<Requirement> {
    let package_of = |id: &ServiceId| catalog.descriptor(id.kind()).map(|d| d.package_kind());
    let mut out = Vec::new();
    for site in all_sites(projects, worktrees) {
        let by = site.id.to_string();
        let env = site.environment();
        out.push(Requirement::new(PackageKind::PHP, Some(env.php.as_series()), None, &by));
        out.push(Requirement::new(PackageKind::COMPOSER, composer_series(env.php.composer_channel()), None, &by));
        for addon in &env.addons {
            out.push(Requirement::new(addon.clone(), None, Some(env.php), &by));
        }
        for id in env.service_ids() {
            if let Some(kind) = package_of(&id) {
                out.push(Requirement::new(kind, id.version(), None, &by));
            }
        }
        if site.served() {
            out.push(Requirement::new(PackageKind::APACHE, None, None, &by));
            out.push(Requirement::new(PackageKind::MOD_FCGID, None, None, &by));
        }
    }
    for s in settings.services.iter().filter(|s| s.always_on) {
        if let Some(kind) = package_of(&s.service) {
            out.push(Requirement::new(kind, s.service.version(), None, SYSTEM));
        }
    }
    if let Some(php) = settings.default_php {
        out.push(Requirement::new(PackageKind::PHP, Some(php.as_series()), None, SYSTEM));
    }
    out
}

/// Distinct requirers of `id` (sorted).
pub(crate) fn used_by(id: &PackageId, requirements: &[Requirement]) -> Vec<String> {
    let mut by: Vec<String> = requirements.iter().filter(|r| r.matches(id)).map(|r| r.required_by.clone()).collect();
    by.sort();
    by.dedup();
    by
}

/// Requirements nothing installed satisfies, de-duplicated by description (sorted).
pub(crate) fn missing(requirements: &[Requirement], installed: &[InstalledPackage]) -> Vec<String> {
    let mut out: Vec<String> = requirements.iter().filter(|r| !r.satisfied_by(installed)).map(Requirement::describe).collect();
    out.sort();
    out.dedup();
    out
}
