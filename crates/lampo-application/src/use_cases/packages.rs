use std::collections::BTreeMap;
use std::sync::Arc;

use lampo_domain::events::DomainEvent;
use lampo_domain::package::{CatalogEntry, InstalledPackage, MergedCatalog, PackageId, PackageKind, PackagePayload, Sha256Digest, latest, latest_in_series, merge_catalogs, verify_checksum};
use lampo_domain::paths::LampoPaths;
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::{
    ArchiveExtractor, EventPublisher, InstalledPackageRegistry, PackageCatalogSource, PackageDownloader, PackageInstallHook, PackageSlotLock, PhpExtensionScanner, ProgressSink,
    ProjectRepository, SettingsRepository, SlotReplacement, VcRuntimeDetector, WorktreeRepository,
};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::vc_runtime::InstalledVcRuntime;
use lampo_domain::version::{PackageVersion, VersionSeries};

use super::environment::EnvironmentSync;
use super::requirements::{requirements, used_by};
use super::resolution::SiteResolver;
use super::service_catalog::ServiceCatalog;
use super::settings_writer::SettingsWriter;
use crate::context::CallContext;
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mapping::package_id_to_dto;

/// Fetches `kind` (every kind each source serves when `None`) from all sources. A failing
/// source is logged and skipped; the call fails (`Network`) only if every relevant source failed.
/// Integrity conflicts between live and pinned hashes are logged and returned with the catalog.
pub(crate) async fn fetch_catalog(sources: &[Arc<dyn PackageCatalogSource>], kind: Option<&PackageKind>) -> AppResult<MergedCatalog> {
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
    let catalog = merge_catalogs(lists);
    for conflict in &catalog.conflicts {
        tracing::warn!(package = %conflict.id, pinned = %conflict.frozen, live = %conflict.live, "a live catalog publishes another SHA-256 than the pinned one; the package will not be installed");
    }
    Ok(catalog)
}

/// `"8.3"` (or `"8.3.14"`) → the 8.3 series.
pub(super) fn parse_series(raw: &str) -> AppResult<VersionSeries> {
    Ok(PackageVersion::parse(raw)?.series())
}

fn installed_dto(p: &InstalledPackage, reqs: &[super::requirements::Requirement]) -> InstalledPackageDto {
    InstalledPackageDto { id: package_id_to_dto(&p.id), path: p.install_dir.display().to_string(), used_by: used_by(&p.id, reqs), series: p.id.series().to_string() }
}

/// Catalog and installed packages (read-only; network for live sources).
pub struct PackageCatalog {
    /// Ordered: earlier sources win on duplicate ids, except for pinned hashes (`merge_catalogs`).
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
        let mut entries = fetch_catalog(&self.sources, kind.as_ref()).await?.entries;
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
        let entries = fetch_catalog(&self.sources, None).await?.entries;
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

/// `lampo package extensions`: [`PhpExtensionsQuery`] for the resolved site's PHP minor (or an
/// explicit one), each extension flagged as enabled or not in the resolved project.
/// Read-only and file-based (the CLI runs it without the app).
pub struct PhpExtensionListing {
    resolver: Arc<SiteResolver>,
    projects: Arc<dyn ProjectRepository>,
    extensions: Arc<PhpExtensionsQuery>,
}

impl PhpExtensionListing {
    pub fn new(resolver: Arc<SiteResolver>, projects: Arc<dyn ProjectRepository>, extensions: Arc<PhpExtensionsQuery>) -> Self {
        Self { resolver, projects, extensions }
    }

    /// Without `php_version`: the PHP minor of the resolved site's project (worktree
    /// environments follow their project), else the global default PHP; `ProjectNotResolved`
    /// when neither exists. With it, resolution only decides the `enabled` flags and never fails
    /// the call. `PackageMissing` if the minor is not installed, `Validation` if malformed.
    pub fn execute(&self, request: &ResolveRequestDto, php_version: Option<&str>) -> AppResult<PhpExtensionListDto> {
        let explicit = php_version.map(|raw| PhpMinor::parse(raw.trim())).transpose()?;
        let resolution = match explicit {
            Some(_) => self.resolver.resolve(request).unwrap_or_else(|e| {
                tracing::debug!("site resolution for `package extensions` failed: {}", e.message);
                None
            }),
            None => Some(self.resolver.resolve(request)?.ok_or_else(|| {
                AppError::new(
                    ErrorCode::ProjectNotResolved,
                    format!("no project for `{}` and no default PHP: pass --php <minor> (installed minors: `lampo package list --kind php --installed`)", request.cwd),
                )
            })?),
        };
        let project = match resolution.as_ref().and_then(|r| r.site.as_ref()) {
            Some(site) => self.projects.get(site.project())?,
            None => None,
        };
        // Without an explicit minor, `resolution` is always `Some`.
        let php = explicit.or(project.as_ref().map(|p| p.environment.php)).or(resolution.as_ref().map(|r| r.target.php)).ok_or_else(|| AppError::internal("no PHP minor to list"))?;
        // Flags only make sense for the project's own PHP build.
        let project = project.filter(|p| p.environment.php == php);
        let extensions = self
            .extensions
            .list(&php.to_string())?
            .into_iter()
            .map(|e| PhpExtensionStateDto { enabled: project.as_ref().map(|p| p.environment.extensions.iter().any(|enabled| enabled.to_string() == e.name)), name: e.name, zend: e.zend })
            .collect();
        Ok(PhpExtensionListDto { php_version: php.to_string(), project_id: project.map(|p| p.id.to_string()), extensions })
    }
}

pub struct InstallPackageDeps {
    pub sources: Vec<Arc<dyn PackageCatalogSource>>,
    pub downloader: Arc<dyn PackageDownloader>,
    pub extractor: Arc<dyn ArchiveExtractor>,
    pub installed: Arc<dyn InstalledPackageRegistry>,
    /// Held for a whole install, so two installs into one slot run one after the other.
    pub slots: Arc<dyn PackageSlotLock>,
    /// Post-install hooks; the one whose `kind()` matches runs.
    pub hooks: Vec<Arc<dyn PackageInstallHook>>,
    pub vc: Arc<dyn VcRuntimeDetector>,
    /// Adopts the first PHP installed as the default PHP.
    pub settings: SettingsWriter,
    pub paths: LampoPaths,
    pub env: Arc<dyn EnvironmentSync>,
    pub events: Arc<dyn EventPublisher>,
}

/// Forwards every report with the same subject (the requested package kind).
struct SubjectProgress<'a> {
    inner: &'a dyn ProgressSink,
    subject: String,
}

impl ProgressSink for SubjectProgress<'_> {
    fn report(&self, progress: Progress) {
        self.inner.report(progress.with_subject(self.subject.clone()));
    }
}

/// Resolve → integrity and VC++ runtime checks → download (hashed while streamed) → verify
/// SHA-256 → extract or place → hook → record → sync.
/// Also used for one-click patch updates (install latest in series; projects bind to minors so
/// they follow).
pub struct InstallPackage {
    deps: InstallPackageDeps,
}

impl InstallPackage {
    pub fn new(deps: InstallPackageDeps) -> Self {
        Self { deps }
    }

    /// Catalog lookup: exact `version`, else newest in `series`, else newest overall. Also returns
    /// the merged catalog, whose integrity conflicts decide whether the entry may be installed.
    async fn resolve(&self, request: &InstallPackageRequest) -> AppResult<(CatalogEntry, MergedCatalog)> {
        let kind = PackageKind::new(request.kind.trim())?;
        let php = request.php_version.as_deref().map(PhpMinor::parse).transpose()?;
        let catalog = fetch_catalog(&self.deps.sources, Some(&kind)).await?;
        let entries = &catalog.entries;
        let found = match (&request.version, &request.series) {
            (Some(v), _) => {
                let version = PackageVersion::parse(v)?;
                entries.iter().find(|e| e.id.kind == kind && e.id.version == version && e.id.php == php)
            }
            (None, Some(s)) => latest_in_series(entries, &kind, parse_series(s)?, php),
            (None, None) => latest(entries, &kind, php),
        };
        let wanted = [Some(kind.to_string()), request.version.clone().or(request.series.clone()), php.map(|p| format!("for PHP {p}"))];
        let entry = found.cloned().ok_or_else(|| AppError::not_found("package", wanted.into_iter().flatten().collect::<Vec<_>>().join(" ")))?;
        Ok((entry, catalog))
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

    /// Download into the downloads dir (hashed while streamed, refused when larger than the
    /// catalog announces), verify, then place the files into the install dir: a corrupt or
    /// oversized download never touches the package currently installed there. The previous
    /// contents of the install dir stay aside in the returned replacement.
    async fn fetch_files(
        &self,
        entry: &CatalogEntry,
        expected: &Sha256Digest,
        install_dir: &std::path::Path,
        progress: &dyn ProgressSink,
        cancel: &CancelFlag,
    ) -> AppResult<SlotReplacement> {
        let d = &self.deps;
        let file = d.paths.downloads_dir().join(format!("{}-{}", entry.id.key(), entry.url.file_name()));
        cancel.check()?;
        let downloaded = d.downloader.download(&entry.url, &file, entry.max_download_bytes(), progress, cancel).await?;
        cancel.check()?;
        progress.report(Progress::new(ProgressStage::Verifying, 0, Some(1)).with_subject(entry.id.key()));
        if let Err(e) = verify_checksum(&file.display().to_string(), expected, &downloaded.sha256) {
            // A corrupt download must not be resumed or reused.
            self.discard(&file).await;
            return Err(e.into());
        }
        progress.report(Progress::new(ProgressStage::Verifying, 1, Some(1)).with_subject(entry.id.key()));
        cancel.check()?;
        let replacement = match &entry.payload {
            PackagePayload::Zip { strip_prefix } => d.extractor.extract_zip(&file, install_dir, strip_prefix.as_deref(), progress, cancel).await?,
            PackagePayload::SingleFile { file_name } => d.extractor.place_file(&file, install_dir, file_name, cancel).await?,
        };
        // Not needed once placed (a single file was moved: only its resume metadata may remain).
        self.discard(&file).await;
        Ok(replacement)
    }

    /// The kind's post-install hook, then the record: what makes the placed files an installed
    /// package. Runs while the previous version is still kept aside.
    async fn configure(&self, package: &InstalledPackage, progress: &dyn ProgressSink, cancel: &CancelFlag) -> AppResult<()> {
        let d = &self.deps;
        cancel.check()?;
        progress.report(Progress::new(ProgressStage::Configuring, 0, Some(1)).with_subject(package.id.key()));
        if let Some(hook) = d.hooks.iter().find(|h| h.kind() == package.id.kind) {
            hook.after_install(package).await?;
        }
        d.installed.record(package)?;
        progress.report(Progress::new(ProgressStage::Configuring, 1, Some(1)).with_subject(package.id.key()));
        Ok(())
    }

    /// Best-effort removal of a download (a leftover only costs disk space).
    async fn discard(&self, file: &std::path::Path) {
        if let Err(e) = self.deps.downloader.discard(file).await {
            tracing::warn!("cannot delete {}: {e}", file.display());
        }
    }

    /// The first PHP installed becomes the default PHP of the CLI shims (a guarded settings save).
    /// Best effort: the package is installed either way, so a failure is only logged (the
    /// default PHP can still be chosen in the settings).
    async fn adopt_default_php(&self, id: &PackageId) {
        if id.kind != PackageKind::PHP {
            return;
        }
        let adopted = self
            .deps
            .settings
            .modify(|current| {
                let mut next = current.clone();
                if next.default_php.is_none() {
                    next.default_php = Some(PhpMinor::try_from(id.series())?);
                }
                Ok(next)
            })
            .await;
        if let Err(e) = adopted {
            tracing::warn!("making {id} the default PHP failed: {}", e.message);
        }
    }

    /// Long-running: the caller runs it as a task (see `crate::tasks`) and passes a progress
    /// sink (GUI events or control-pipe frames). Refuses entries without a checksum or whose live
    /// and pinned checksums disagree (`ChecksumMismatch`), and builds
    /// whose VC++ runtime is missing (`PackageMissing`, `details.downloadUrl`). Already installed
    /// → returned as is. Sets `default_php` when installing the first PHP (best effort). Publishes
    /// `PackagesChanged`. A failure after the files were placed (hook, record) puts the
    /// previous version of the slot back, or removes the new files if there was none.
    ///
    /// Installs into the same slot (kind + series + PHP minor) run one at a time, across the GUI
    /// and the CLI: a second request waits for the first, then finds the package installed.
    ///
    /// The final environment sync is best effort: its failure is logged, not returned (the
    /// package *is* installed; `lampo up` reports the environment problem).
    pub async fn execute(&self, request: InstallPackageRequest, ctx: &CallContext, progress: &dyn ProgressSink, cancel: &CancelFlag) -> AppResult<InstalledPackageDto> {
        let d = &self.deps;
        // One subject for every stage, so a UI can follow one package through resolve,
        // download (the downloader would name the file), extraction (unnamed) and configure.
        let progress = &SubjectProgress { inner: progress, subject: request.kind.clone() };
        progress.report(Progress::new(ProgressStage::Resolving, 0, None));
        let (entry, catalog) = self.resolve(&request).await?;
        let slot = d.slots.acquire(&entry.id, cancel).await?;
        if let Some(existing) = d.installed.list()?.into_iter().find(|p| p.id == entry.id) {
            // Settings may have been reset while packages stayed: still adopt it as the default.
            self.adopt_default_php(&existing.id).await;
            return Ok(InstalledPackageDto {
                id: package_id_to_dto(&existing.id),
                path: existing.install_dir.display().to_string(),
                used_by: Vec::new(),
                series: existing.id.series().to_string(),
            });
        }
        catalog.ensure_consistent(&entry.id)?;
        self.check_vc_runtime(&entry)?;
        let expected = entry
            .sha256
            .ok_or_else(|| AppError::new(ErrorCode::ChecksumMismatch, format!("{} has no published SHA-256 checksum and cannot be verified, so it is not installed", entry.id)))?;

        let install_dir = d.installed.install_dir(&entry.id);
        let package = InstalledPackage::new(entry.id.clone(), install_dir.clone(), entry.vc_runtime);
        let replacement = self.fetch_files(&entry, &expected, &install_dir, progress, cancel).await?;
        if let Err(e) = self.configure(&package, progress, cancel).await {
            if let Err(rollback) = d.extractor.roll_back(replacement).await {
                tracing::error!("restoring {} after a failed install of {} failed: {rollback}", install_dir.display(), entry.id);
            }
            return Err(e);
        }
        d.extractor.commit(replacement).await;
        drop(slot);

        self.adopt_default_php(&entry.id).await;
        d.events.publish(DomainEvent::PackagesChanged);
        if let Err(e) = d.env.sync(ctx).await {
            tracing::warn!("environment sync after installing {} failed: {}", entry.id, e.message);
        }
        Ok(InstalledPackageDto { id: package_id_to_dto(&package.id), path: package.install_dir.display().to_string(), used_by: Vec::new(), series: package.id.series().to_string() })
    }
}
