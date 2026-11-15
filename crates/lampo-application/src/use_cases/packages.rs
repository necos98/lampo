use std::collections::BTreeMap;
use std::sync::Arc;

use lampo_domain::events::DomainEvent;
use lampo_domain::package::{CatalogEntry, InstalledPackage, PackageId, PackageKind, PackagePayload, Sha256Digest, latest, latest_in_series, merge_catalogs, verify_checksum};
use lampo_domain::paths::LampoPaths;
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::{
    ArchiveExtractor, ChecksumCalculator, EventPublisher, InstalledPackageRegistry, PackageCatalogSource, PackageDownloader, PackageInstallHook, PhpExtensionScanner, ProgressSink,
    ProjectRepository, SettingsRepository, VcRuntimeDetector, WorktreeRepository,
};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::vc_runtime::InstalledVcRuntime;
use lampo_domain::version::{PackageVersion, VersionSeries};

use super::environment::{EnvironmentSync, GeneratedFilesWriter};
use super::requirements::{requirements, used_by};
use super::service_catalog::ServiceCatalog;
use crate::context::CallContext;
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mapping::{package_id_from_dto, package_id_to_dto};

/// Fetches `kind` (every kind each source serves when `None`) from all sources. A failing
/// source is logged and skipped; the call fails (`Network`) only if every relevant source failed.
pub(crate) async fn fetch_catalog(sources: &[Arc<dyn PackageCatalogSource>], kind: Option<&PackageKind>) -> AppResult<Vec<CatalogEntry>> {
    let mut lists = Vec::new();
    let mut failures = Vec::new();
    let mut attempted = 0;
    for source in sources {
        let kinds: Vec<PackageKind> = match kind {
            Some(k) if source.kinds().contains(k) => vec![k.clone()],
            Some(_) => continue,
            None => source.kinds(),
        };
        for k in kinds {
            attempted += 1;
            match source.fetch(&k).await {
                Ok(entries) => lists.push(entries),
                Err(e) => {
                    tracing::warn!("catalog source `{}` failed for {k}: {e}", source.name());
                    failures.push(format!("{}: {e}", source.name()));
                }
            }
        }
    }
    if attempted > 0 && lists.is_empty() {
        return Err(AppError::new(ErrorCode::Network, format!("no package catalog is reachable ({})", failures.join("; "))));
    }
    Ok(merge_catalogs(lists))
}

/// `"8.3"` (or `"8.3.14"`) → the 8.3 series.
fn parse_series(raw: &str) -> AppResult<VersionSeries> {
    Ok(PackageVersion::parse(raw)?.series())
}

fn installed_dto(p: &InstalledPackage, reqs: &[super::requirements::Requirement]) -> InstalledPackageDto {
    InstalledPackageDto { id: package_id_to_dto(&p.id), path: p.install_dir.display().to_string(), used_by: used_by(&p.id, reqs), series: p.id.series().to_string() }
}

/// Catalog and installed packages (read-only; network for live sources).
pub struct PackageCatalog {
    /// Ordered: earlier sources win on duplicate ids (`merge_catalogs`).
    sources: Vec<Arc<dyn PackageCatalogSource>>,
    installed: Arc<dyn InstalledPackageRegistry>,
    vc: Arc<dyn VcRuntimeDetector>,
    projects: Arc<dyn ProjectRepository>,
    worktrees: Arc<dyn WorktreeRepository>,
    settings: Arc<dyn SettingsRepository>,
    services: Arc<ServiceCatalog>,
}

pub struct PackageCatalogDeps {
    pub sources: Vec<Arc<dyn PackageCatalogSource>>,
    pub installed: Arc<dyn InstalledPackageRegistry>,
    pub vc: Arc<dyn VcRuntimeDetector>,
    pub projects: Arc<dyn ProjectRepository>,
    pub worktrees: Arc<dyn WorktreeRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub services: Arc<ServiceCatalog>,
}

impl PackageCatalog {
    pub fn new(deps: PackageCatalogDeps) -> Self {
        let PackageCatalogDeps { sources, installed, vc, projects, worktrees, settings, services } = deps;
        Self { sources, installed, vc, projects, worktrees, settings, services }
    }

    /// The GUI tab of a package: PHP-pinned add-ons, PHP itself, the service kind whose
    /// package it is, or `tools`.
    fn group_of(&self, id: &PackageId) -> String {
        if id.php.is_some() {
            return "addons".into();
        }
        if id.kind == PackageKind::PHP {
            return "php".into();
        }
        match self.services.descriptors().iter().find(|d| d.package_kind() == id.kind) {
            Some(d) => d.kind().to_string(),
            None => "tools".into(),
        }
    }

    /// Catalog entries of `kind` (all kinds when `None`), newest first within each kind and
    /// series, with their series and GUI group. A failing live source degrades to the
    /// other sources (logged), it does not fail the call.
    pub async fn list(&self, kind: Option<&str>) -> AppResult<Vec<CatalogEntryDto>> {
        let kind = kind.map(|k| PackageKind::new(k.trim())).transpose()?;
        let mut entries = fetch_catalog(&self.sources, kind.as_ref()).await?;
        // Newest first within a kind and series, so front-ends never re-derive version ordering.
        entries.sort_by(|a, b| {
            (&a.id.kind, std::cmp::Reverse(a.id.series()), std::cmp::Reverse(a.id.php), std::cmp::Reverse(&a.id.version)).cmp(&(
                &b.id.kind,
                std::cmp::Reverse(b.id.series()),
                std::cmp::Reverse(b.id.php),
                std::cmp::Reverse(&b.id.version),
            ))
        });
        let installed = self.installed.list()?;
        let runtimes: Option<Vec<InstalledVcRuntime>> = self.vc.installed().map_err(|e| tracing::warn!("cannot read installed VC++ runtimes: {e}")).ok();
        Ok(entries
            .iter()
            .map(|e| CatalogEntryDto {
                id: package_id_to_dto(&e.id),
                channel: match e.channel {
                    lampo_domain::package::CatalogChannel::Frozen => CatalogChannelDto::Frozen,
                    lampo_domain::package::CatalogChannel::Live => CatalogChannelDto::Live,
                },
                end_of_life: e.end_of_life,
                size_bytes: e.size_bytes,
                vc_runtime: e.vc_runtime.map(|r| r.to_string()),
                vc_runtime_installed: e.vc_runtime.and_then(|r| runtimes.as_ref().map(|rt| r.is_satisfied_by(rt))),
                installed: installed.iter().any(|p| p.id == e.id),
                series: e.id.series().to_string(),
                group: self.group_of(&e.id),
            })
            .collect())
    }

    pub fn list_installed(&self) -> AppResult<Vec<InstalledPackageDto>> {
        let reqs = requirements(&self.projects.list()?, &self.worktrees.list_all()?, &self.settings.load()?, &self.services);
        let mut installed = self.installed.list()?;
        installed.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(installed.iter().map(|p| installed_dto(p, &reqs)).collect())
    }

    /// Newer patch available in the same series for each installed series.
    pub async fn check_updates(&self) -> AppResult<Vec<UpdateAvailableDto>> {
        let installed = self.installed.list()?;
        let mut newest: BTreeMap<(PackageKind, VersionSeries, Option<PhpMinor>), &InstalledPackage> = BTreeMap::new();
        for p in &installed {
            let key = (p.id.kind.clone(), p.id.series(), p.id.php);
            if newest.get(&key).is_none_or(|cur| cur.id.version < p.id.version) {
                newest.insert(key, p);
            }
        }
        let entries = fetch_catalog(&self.sources, None).await?;
        Ok(newest
            .into_iter()
            .filter_map(|((kind, series, php), current)| {
                let candidate = latest_in_series(&entries, &kind, series, php)?;
                (candidate.id.version > current.id.version).then(|| UpdateAvailableDto {
                    kind: kind.to_string(),
                    series: series.to_string(),
                    php_version: php.map(|p| p.to_string()),
                    installed: current.id.version.to_string(),
                    latest: candidate.id.version.to_string(),
                })
            })
            .collect())
    }
}

/// The extensions an installed PHP minor ships, for the project editor's checkboxes.
pub struct PhpExtensionsQuery {
    installed: Arc<dyn InstalledPackageRegistry>,
    scanner: Arc<dyn PhpExtensionScanner>,
}

impl PhpExtensionsQuery {
    pub fn new(installed: Arc<dyn InstalledPackageRegistry>, scanner: Arc<dyn PhpExtensionScanner>) -> Self {
        Self { installed, scanner }
    }

    /// Extensions of the newest installed build of `php_version` (`"8.3"`), sorted.
    /// `PackageMissing` if that minor is not installed.
    pub fn list(&self, php_version: &str) -> AppResult<Vec<PhpExtensionDto>> {
        let php = PhpMinor::parse(php_version.trim())?;
        let dir = super::support::php_dir(&self.installed.list()?, php).ok_or_else(|| super::support::package_missing(format!("PHP {php}")))?;
        Ok(self.scanner.bundled_extensions(&dir)?.into_iter().map(|name| PhpExtensionDto { zend: lampo_domain::web::is_zend_extension(&name), name: name.to_string() }).collect())
    }
}

pub struct InstallPackageDeps {
    pub sources: Vec<Arc<dyn PackageCatalogSource>>,
    pub downloader: Arc<dyn PackageDownloader>,
    pub checksum: Arc<dyn ChecksumCalculator>,
    pub extractor: Arc<dyn ArchiveExtractor>,
    pub installed: Arc<dyn InstalledPackageRegistry>,
    /// Post-install hooks; the one whose `kind()` matches runs.
    pub hooks: Vec<Arc<dyn PackageInstallHook>>,
    pub vc: Arc<dyn VcRuntimeDetector>,
    pub settings: Arc<dyn SettingsRepository>,
    pub paths: LampoPaths,
    pub env: Arc<dyn EnvironmentSync>,
    pub events: Arc<dyn EventPublisher>,
}

/// Resolve → VC++ runtime check → download → verify SHA-256 → extract → hook → record → sync.
/// Also used for one-click patch updates (install latest in series; projects bind to minors so
/// they follow).
pub struct InstallPackage {
    deps: InstallPackageDeps,
}

impl InstallPackage {
    pub fn new(deps: InstallPackageDeps) -> Self {
        Self { deps }
    }

    /// Catalog lookup: exact `version`, else newest in `series`, else newest overall.
    async fn resolve(&self, request: &InstallPackageRequest) -> AppResult<CatalogEntry> {
        let kind = PackageKind::new(request.kind.trim())?;
        let php = request.php_version.as_deref().map(PhpMinor::parse).transpose()?;
        let entries = fetch_catalog(&self.deps.sources, Some(&kind)).await?;
        let found = match (&request.version, &request.series) {
            (Some(v), _) => {
                let version = PackageVersion::parse(v)?;
                entries.iter().find(|e| e.id.kind == kind && e.id.version == version && e.id.php == php)
            }
            (None, Some(s)) => latest_in_series(&entries, &kind, parse_series(s)?, php),
            (None, None) => latest(&entries, &kind, php),
        };
        let wanted = [Some(kind.to_string()), request.version.clone().or(request.series.clone()), php.map(|p| format!("for PHP {p}"))];
        found.cloned().ok_or_else(|| AppError::not_found("package", wanted.into_iter().flatten().collect::<Vec<_>>().join(" ")))
    }

    fn check_vc_runtime(&self, entry: &CatalogEntry) -> AppResult<()> {
        let Some(runtime) = entry.vc_runtime else { return Ok(()) };
        if runtime.is_satisfied_by(&self.deps.vc.installed()?) {
            return Ok(());
        }
        let redist = runtime.redistributable();
        Err(AppError::new(ErrorCode::PackageMissing, format!("{} needs the {redist} runtime, which is not installed", entry.id)).with_details(serde_json::json!({
            "redistributable": redist.to_string(),
            "downloadUrl": redist.download_url(),
            "runtime": runtime.to_string(),
        })))
    }

    /// Download, verify and place the files. Single-file payloads are downloaded straight into
    /// the install dir (there is no copy port).
    async fn fetch_files(&self, entry: &CatalogEntry, expected: &Sha256Digest, install_dir: &std::path::Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> AppResult<()> {
        let d = &self.deps;
        let file = match &entry.payload {
            PackagePayload::Zip { .. } => d.paths.downloads_dir().join(format!("{}-{}", entry.id.key(), entry.url.file_name())),
            PackagePayload::SingleFile { file_name } => install_dir.join(file_name),
        };
        cancel.check()?;
        d.downloader.download(&entry.url, &file, progress, cancel).await?;
        cancel.check()?;
        progress.report(Progress::new(ProgressStage::Verifying, 0, Some(1)).with_subject(entry.id.key()));
        let actual = d.checksum.sha256(&file).await?;
        if let Err(e) = verify_checksum(&file.display().to_string(), expected, &actual) {
            // A corrupt download must not be resumed or reused.
            self.discard(&file).await;
            return Err(e.into());
        }
        progress.report(Progress::new(ProgressStage::Verifying, 1, Some(1)).with_subject(entry.id.key()));
        cancel.check()?;
        if let PackagePayload::Zip { strip_prefix } = &entry.payload {
            d.extractor.extract_zip(&file, install_dir, strip_prefix.as_deref(), progress, cancel).await?;
            // The archive is not needed once extracted.
            self.discard(&file).await;
        }
        Ok(())
    }

    /// Best-effort removal of a download (a leftover only costs disk space).
    async fn discard(&self, file: &std::path::Path) {
        if let Err(e) = self.deps.downloader.discard(file).await {
            tracing::warn!("cannot delete {}: {e}", file.display());
        }
    }

    /// The first PHP installed becomes the default PHP of the CLI shims.
    fn adopt_default_php(&self, id: &PackageId) -> AppResult<()> {
        if id.kind != PackageKind::PHP {
            return Ok(());
        }
        let mut settings = self.deps.settings.load()?;
        if settings.default_php.is_some() {
            return Ok(());
        }
        settings.default_php = Some(PhpMinor::try_from(id.series())?);
        self.deps.settings.save(&settings)?;
        self.deps.events.publish(DomainEvent::SettingsChanged);
        Ok(())
    }

    /// Long-running: the caller runs it as a task (see `crate::tasks`) and passes a progress
    /// sink (GUI events or control-pipe frames). Refuses entries without a checksum and builds
    /// whose VC++ runtime is missing (`PackageMissing`, `details.downloadUrl`). Already installed
    /// → returned as is. Sets `default_php` when installing the first PHP. Publishes
    /// `PackagesChanged`. A failure after the files were placed removes them again.
    ///
    /// The final environment sync is best effort: its failure is logged, not returned (the
    /// package *is* installed; `lampo up` reports the environment problem).
    pub async fn execute(&self, request: InstallPackageRequest, ctx: &CallContext, progress: &dyn ProgressSink, cancel: &CancelFlag) -> AppResult<InstalledPackageDto> {
        let d = &self.deps;
        progress.report(Progress::new(ProgressStage::Resolving, 0, None).with_subject(request.kind.clone()));
        let entry = self.resolve(&request).await?;
        if let Some(existing) = d.installed.list()?.into_iter().find(|p| p.id == entry.id) {
            // Settings may have been reset while packages stayed: still adopt it as the default.
            self.adopt_default_php(&existing.id)?;
            return Ok(InstalledPackageDto {
                id: package_id_to_dto(&existing.id),
                path: existing.install_dir.display().to_string(),
                used_by: Vec::new(),
                series: existing.id.series().to_string(),
            });
        }
        self.check_vc_runtime(&entry)?;
        let expected = entry
            .sha256
            .ok_or_else(|| AppError::new(ErrorCode::ChecksumMismatch, format!("{} has no published SHA-256 checksum and cannot be verified, so it is not installed", entry.id)))?;

        let install_dir = d.installed.install_dir(&entry.id);
        let package = InstalledPackage::new(entry.id.clone(), install_dir.clone(), entry.vc_runtime);
        let placed = async {
            self.fetch_files(&entry, &expected, &install_dir, progress, cancel).await?;
            cancel.check()?;
            progress.report(Progress::new(ProgressStage::Configuring, 0, Some(1)).with_subject(entry.id.key()));
            if let Some(hook) = d.hooks.iter().find(|h| h.kind() == entry.id.kind) {
                hook.after_install(&package).await?;
            }
            d.installed.record(&package)?;
            progress.report(Progress::new(ProgressStage::Configuring, 1, Some(1)).with_subject(entry.id.key()));
            AppResult::Ok(())
        }
        .await;
        if let Err(e) = placed {
            if let Err(cleanup) = d.installed.remove(&entry.id) {
                tracing::debug!("cleanup of {} after a failed install: {cleanup}", entry.id);
            }
            return Err(e);
        }

        self.adopt_default_php(&entry.id)?;
        d.events.publish(DomainEvent::PackagesChanged);
        if let Err(e) = d.env.sync(ctx).await {
            tracing::warn!("environment sync after installing {} failed: {}", entry.id, e.message);
        }
        Ok(InstalledPackageDto { id: package_id_to_dto(&package.id), path: package.install_dir.display().to_string(), used_by: Vec::new(), series: package.id.series().to_string() })
    }
}

pub struct UninstallPackage {
    installed: Arc<dyn InstalledPackageRegistry>,
    projects: Arc<dyn ProjectRepository>,
    worktrees: Arc<dyn WorktreeRepository>,
    settings: Arc<dyn SettingsRepository>,
    events: Arc<dyn EventPublisher>,
    /// Regenerates the shim index / php.ini files after a removal, so they never point at
    /// deleted binaries (the app's `ApplyEnvironment`).
    files: Arc<dyn GeneratedFilesWriter>,
    services: Arc<ServiceCatalog>,
}

pub struct UninstallPackageDeps {
    pub installed: Arc<dyn InstalledPackageRegistry>,
    pub projects: Arc<dyn ProjectRepository>,
    pub worktrees: Arc<dyn WorktreeRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub events: Arc<dyn EventPublisher>,
    pub files: Arc<dyn GeneratedFilesWriter>,
    pub services: Arc<ServiceCatalog>,
}

impl UninstallPackage {
    pub fn new(deps: UninstallPackageDeps) -> Self {
        let UninstallPackageDeps { installed, projects, worktrees, settings, events, files, services } = deps;
        Self { installed, projects, worktrees, settings, events, files, services }
    }

    /// `Conflict` if it is the last installed build satisfying something in use (a site's PHP
    /// minor, MySQL series, add-on…, the always-on MySQL or the default PHP). `NotFound` if not
    /// installed.
    pub fn execute(&self, id: PackageIdDto) -> AppResult<()> {
        let id = package_id_from_dto(&id)?;
        let installed = self.installed.list()?;
        if !installed.iter().any(|p| p.id == id) {
            return Err(AppError::not_found("package", &id));
        }
        let others: Vec<InstalledPackage> = installed.into_iter().filter(|p| p.id != id).collect();
        let reqs = requirements(&self.projects.list()?, &self.worktrees.list_all()?, &self.settings.load()?, &self.services);
        let mut blocking: Vec<String> = reqs.iter().filter(|r| r.matches(&id) && !r.satisfied_by(&others)).map(|r| r.required_by.clone()).collect();
        blocking.sort();
        blocking.dedup();
        if !blocking.is_empty() {
            return Err(AppError::new(ErrorCode::Conflict, format!("{id} is in use by {}", blocking.join(", "))).with_details(serde_json::json!({ "usedBy": blocking })));
        }
        self.installed.remove(&id)?;
        self.events.publish(DomainEvent::PackagesChanged);
        self.files.write_generated_files()
    }
}
