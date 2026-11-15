use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lampo_domain::package::{PackageKind, latest};
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::{AppLocationRecorder, InstalledPackageRegistry, PackageCatalogSource, ProgressSink, ServiceRunner, SettingsRepository};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::service::ServiceState;
use lampo_domain::version::VersionSeries;

use super::environment::EnvironmentSync;
use super::generation::runner_for;
use super::service_catalog::ServiceCatalog;
use super::packages::{InstallPackage, fetch_catalog};
use super::claude_code::ClaudeCodeSetup;
use super::shims::ShimSetup;
use super::support::{COMPOSER_LTS_SERIES, installed_php_minors, newest_of};
use crate::context::{CallContext, ElevationPolicy, Origin};
use crate::dto::{FirstRunItemDto, FirstRunStatusDto, InstallPackageRequest};
use crate::error::{AppError, AppResult};

/// App start: record where the app is (for the CLI copy in the shim dir), refresh the shims
/// and the CLI copy left behind by an app update, regenerate files and bring
/// services to the plan (always-on MySQL included). Runs with `ElevationPolicy::Deny` — a
/// pending hosts change is surfaced in the GUI instead of prompting at boot.
pub struct Startup {
    env: Arc<dyn EnvironmentSync>,
    settings: Arc<dyn SettingsRepository>,
    shims: Arc<ShimSetup>,
    app_location: Arc<dyn AppLocationRecorder>,
    claude_code: Option<Arc<ClaudeCodeSetup>>,
}

impl Startup {
    pub fn new(env: Arc<dyn EnvironmentSync>, settings: Arc<dyn SettingsRepository>, shims: Arc<ShimSetup>, app_location: Arc<dyn AppLocationRecorder>) -> Self {
        Self { env, settings, shims, app_location, claude_code: None }
    }

    /// Also updates an outdated Claude Code plugin (only one the user installed before).
    pub fn with_claude_code(mut self, claude_code: Arc<ClaudeCodeSetup>) -> Self {
        self.claude_code = Some(claude_code);
        self
    }

    /// App location and shims first (failures are only logged: the old files still work). The sync is a no-op
    /// until the first-run wizard completed; its errors (pending hosts change, busy port, …) are
    /// returned for the GUI to show; everything else was still applied. The Claude Code plugin
    /// update (after the CLI copy is refreshed: the hook calls it) runs concurrently with the
    /// sync, so its `claude` processes never delay the services; it only logs.
    pub async fn run(&self) -> AppResult<()> {
        if let Err(e) = self.app_location.record_running_app() {
            tracing::warn!("recording the app location failed: {e}");
        }
        match self.shims.refresh() {
            Ok(true) => tracing::info!("shims refreshed to the current lampo-shim.exe"),
            Ok(false) => {}
            Err(e) => tracing::warn!("refreshing the shims failed: {e}"),
        }
        let integrations = async {
            if let Some(claude_code) = &self.claude_code {
                claude_code.refresh_if_outdated().await;
            }
        };
        let ((), synced) = tokio::join!(integrations, self.sync());
        synced
    }

    async fn sync(&self) -> AppResult<()> {
        if !self.settings.load()?.first_run_completed {
            return Ok(());
        }
        self.env.sync(&CallContext { origin: Origin::Gui, elevation: ElevationPolicy::Deny }).await.map(|_| ())
    }
}

/// Closed while the app is going away (quit, or an update about to replace it): front-ends
/// that could start services again (the control pipe) refuse mutating requests meanwhile.
#[derive(Debug, Default)]
pub struct ShutdownGate(AtomicBool);

impl ShutdownGate {
    pub fn is_closed(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    fn set(&self, closed: bool) {
        self.0.store(closed, Ordering::SeqCst);
    }
}

/// App exit (tray "Quit") and app update (before the installer replaces the app): stop every
/// service gracefully. The Job Object kills leftovers anyway. Does not exit the process.
pub struct Shutdown {
    runners: Vec<Arc<dyn ServiceRunner>>,
    catalog: Arc<ServiceCatalog>,
    settings: Arc<dyn SettingsRepository>,
    gate: Arc<ShutdownGate>,
}

impl Shutdown {
    /// `settings` + `catalog` tell which services are configured (every MySQL series, every kind).
    pub fn new(runners: Vec<Arc<dyn ServiceRunner>>, catalog: Arc<ServiceCatalog>, settings: Arc<dyn SettingsRepository>) -> Self {
        Self { runners, catalog, settings, gate: Arc::default() }
    }

    /// Closed by [`Shutdown::run`], for the front-ends to check.
    pub fn gate(&self) -> Arc<ShutdownGate> {
        self.gate.clone()
    }

    /// The app is not going away after all (the update could not start): accept requests again.
    /// Services stay stopped until something brings them up (`Startup::run`, the user).
    pub fn cancel(&self) {
        self.gate.set(false);
    }

    /// Closes the gate, then stops Apache first and the rest of the configured services. Tries
    /// every service; returns the first error. Unreadable settings are an error (the Job Object
    /// still kills whatever is left when the app exits).
    pub async fn run(&self) -> AppResult<()> {
        self.gate.set(true);
        let ids = self.catalog.configured(&self.settings.load()?);
        let mut first_error: Option<AppError> = None;
        for id in ids {
            let Some(runner) = runner_for(&self.runners, id.kind()) else { continue };
            if matches!(runner.status(&id).state, ServiceState::Stopped) {
                continue;
            }
            if let Err(e) = runner.stop(&id).await {
                tracing::warn!("stopping {id} failed: {e}");
                first_error.get_or_insert(e.into());
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

/// One thing the first-run wizard installs.
struct Item {
    kind: PackageKind,
    series: Option<VersionSeries>,
}

impl Item {
    fn new(kind: PackageKind, series: Option<VersionSeries>) -> Self {
        Self { kind, series }
    }
}

/// First-run wizard: Apache + mod_fcgid, latest PHP, MySQL default series, Composer (stable and
/// 2.2 LTS when needed), then shims, then `first_run_completed = true`.
pub struct FirstRunSetup {
    settings: Arc<dyn SettingsRepository>,
    sources: Vec<Arc<dyn PackageCatalogSource>>,
    installed: Arc<dyn InstalledPackageRegistry>,
    install: Arc<InstallPackage>,
    env: Arc<dyn EnvironmentSync>,
    /// Installs the shims (and puts them on the user PATH) as the wizard's last step.
    shims: Arc<ShimSetup>,
}

pub struct FirstRunDeps {
    pub settings: Arc<dyn SettingsRepository>,
    pub sources: Vec<Arc<dyn PackageCatalogSource>>,
    pub installed: Arc<dyn InstalledPackageRegistry>,
    pub install: Arc<InstallPackage>,
    pub env: Arc<dyn EnvironmentSync>,
    pub shims: Arc<ShimSetup>,
}

impl FirstRunSetup {
    pub fn new(deps: FirstRunDeps) -> Self {
        let FirstRunDeps { settings, sources, installed, install, env, shims } = deps;
        Self { settings, sources, installed, install, env, shims }
    }

    /// The wizard's items. Composer 2.2 LTS is listed once a PHP older than 7.2 is installed.
    fn items(&self) -> AppResult<Vec<(Item, bool)>> {
        let settings = self.settings.load()?;
        let installed = self.installed.list()?;
        let mut items = vec![
            Item::new(PackageKind::APACHE, None),
            Item::new(PackageKind::MOD_FCGID, None),
            Item::new(PackageKind::PHP, None),
            Item::new(PackageKind::MYSQL, Some(settings.default_mysql().as_series())),
            Item::new(PackageKind::COMPOSER, None),
        ];
        let lts_needed = installed_php_minors(&installed).iter().any(|php| php.composer_channel() == lampo_domain::php::ComposerChannel::Lts22);
        if lts_needed {
            items.push(Item::new(PackageKind::COMPOSER, Some(COMPOSER_LTS_SERIES)));
        }
        Ok(items
            .into_iter()
            .map(|item| {
                let done = newest_of(&installed, &item.kind, item.series, None).is_some();
                (item, done)
            })
            .collect())
    }

    pub fn status(&self) -> AppResult<FirstRunStatusDto> {
        Ok(FirstRunStatusDto {
            completed: self.settings.load()?.first_run_completed,
            items: self.items()?.iter().map(|(i, done)| FirstRunItemDto { kind: i.kind.to_string(), series: i.series.map(|s| s.to_string()), installed: *done }).collect(),
        })
    }

    /// Newest PHP minor offered by the catalogs (falls back to "latest" resolution by the
    /// install use case when the catalogs cannot tell).
    async fn latest_php(&self) -> Option<VersionSeries> {
        let entries = fetch_catalog(&self.sources, Some(&PackageKind::PHP)).await.ok()?.entries;
        let newest = latest(&entries, &PackageKind::PHP, None)?;
        PhpMinor::try_from(newest.id.series()).ok().map(PhpMinor::as_series)
    }

    /// Installs what is missing (a task: progress per item). Resumable: re-running skips
    /// installed items.
    pub async fn run(&self, ctx: &CallContext, progress: &dyn ProgressSink, cancel: &CancelFlag) -> AppResult<FirstRunStatusDto> {
        let missing: Vec<Item> = self.items()?.into_iter().filter(|(_, done)| !done).map(|(i, _)| i).collect();
        let total = missing.len() as u64;
        for (n, item) in missing.into_iter().enumerate() {
            cancel.check()?;
            let series = match item.series {
                Some(s) => Some(s),
                None if item.kind == PackageKind::PHP => self.latest_php().await,
                None => None,
            };
            progress.report(Progress::new(ProgressStage::Resolving, n as u64, Some(total)).with_subject(item.kind.to_string()));
            let request = InstallPackageRequest { kind: item.kind.to_string(), version: None, series: series.map(|s| s.to_string()), php_version: None };
            self.install.execute(request, ctx, progress, cancel).await?;
        }
        cancel.check()?;
        self.shims.install()?;
        let mut settings = self.settings.load()?;
        if !settings.first_run_completed {
            settings.first_run_completed = true;
            self.settings.save(&settings)?;
        }
        self.env.sync(ctx).await?;
        self.status()
    }
}
