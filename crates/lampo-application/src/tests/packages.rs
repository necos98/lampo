//! Package catalog, install pipeline, uninstall rules, first run.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use lampo_domain::events::DomainEvent;
use lampo_domain::package::{PackageKind, PackagePayload};
use lampo_domain::ports::{InstalledPackageRegistry, PackageCatalogSource, ProjectRepository, SettingsRepository};
use lampo_domain::progress::{CancelFlag, ProgressStage};
use lampo_domain::vc_runtime::VcRuntime;

use super::{cli, gui};
use crate::dto::*;
use crate::error::ErrorCode;
use crate::mapping::package_id_to_dto;
use crate::testing::*;
use crate::use_cases::*;

struct Kit {
    w: World,
    source: Arc<FakeSource>,
    live: Arc<FakeSource>,
    downloader: Arc<FakeDownloader>,
    checksum: Arc<FakeChecksum>,
    extractor: Arc<FakeExtractor>,
    hook: Arc<FakeHook>,
    vc: Arc<FakeVc>,
    sync: Arc<CountingSync>,
}

impl Kit {
    fn new(w: World) -> Self {
        let mut php84 = catalog_entry(PackageKind::PHP, "8.4.2", None);
        php84.vc_runtime = Some(VcRuntime::Vs17);
        let mut composer = catalog_entry(PackageKind::COMPOSER, "2.8.4", None);
        composer.payload = PackagePayload::SingleFile { file_name: "composer.phar".into() };
        let source = FakeSource::new(
            "embedded",
            vec![
                catalog_entry(PackageKind::APACHE, "2.4.62", None),
                catalog_entry(PackageKind::MOD_FCGID, "2.3.10", None),
                catalog_entry(PackageKind::PHP, "8.3.14", None),
                catalog_entry(PackageKind::PHP, "8.3.15", None),
                php84,
                catalog_entry(PackageKind::PHP, "5.6.40", None),
                catalog_entry(PackageKind::MYSQL, "8.4.3", None),
                catalog_entry(PackageKind::MYSQL, "5.7.44", None),
                composer,
                catalog_entry(PackageKind::COMPOSER, "2.2.25", None),
                catalog_entry(PackageKind::XDEBUG, "3.3.2", Some("8.3")),
            ],
        );
        let live = FakeSource::new("windows.php.net", vec![catalog_entry(PackageKind::PHP, "8.4.3", None)]);
        Self {
            w,
            source,
            live,
            downloader: Arc::new(FakeDownloader::default()),
            checksum: Arc::new(FakeChecksum(Mutex::new(digest(7)))),
            extractor: Arc::new(FakeExtractor::default()),
            hook: Arc::new(FakeHook { kind: PackageKind::PHP, calls: Mutex::default() }),
            vc: Arc::new(FakeVc::modern()),
            sync: Arc::new(CountingSync::default()),
        }
    }

    fn sources(&self) -> Vec<Arc<dyn PackageCatalogSource>> {
        vec![self.live.clone(), self.source.clone()]
    }

    fn install(&self) -> Arc<InstallPackage> {
        Arc::new(InstallPackage::new(InstallPackageDeps {
            sources: self.sources(),
            downloader: self.downloader.clone(),
            checksum: self.checksum.clone(),
            extractor: self.extractor.clone(),
            installed: self.w.installed.clone(),
            hooks: vec![self.hook.clone()],
            vc: self.vc.clone(),
            settings: self.w.settings.clone(),
            paths: self.w.paths.clone(),
            env: self.sync.clone(),
            events: self.w.events.clone(),
        }))
    }

    fn first_run(&self, shims: Arc<ShimSetup>) -> FirstRunSetup {
        FirstRunSetup::new(FirstRunDeps {
            settings: self.w.settings.clone(),
            sources: self.sources(),
            installed: self.w.installed.clone(),
            install: self.install(),
            env: self.sync.clone(),
            shims,
        })
    }

    fn catalog(&self) -> PackageCatalog {
        PackageCatalog::new(PackageCatalogDeps {
            sources: self.sources(),
            installed: self.w.installed.clone(),
            vc: self.vc.clone(),
            projects: self.w.projects.clone(),
            worktrees: self.w.worktrees.clone(),
            settings: self.w.settings.clone(),
            services: self.w.services.clone(),
        })
    }
}

fn request(kind: &str, version: Option<&str>, series: Option<&str>) -> InstallPackageRequest {
    InstallPackageRequest { kind: kind.into(), version: version.map(Into::into), series: series.map(Into::into), php_version: None }
}

#[tokio::test]
async fn install_runs_the_whole_pipeline_and_adopts_the_first_php_as_default() {
    let k = Kit::new(World::new());
    let progress = RecordingProgress::default();
    let dto = k.install().execute(request("php", None, Some("8.3")), &gui(), &progress, &CancelFlag::new()).await.unwrap();
    assert_eq!(dto.id, PackageIdDto { kind: "php".into(), version: "8.3.15".into(), php_version: None });
    assert_eq!(dto.path, "C:\\lampo\\packages\\php-8.3.15");
    assert_eq!(progress.stages(), [ProgressStage::Resolving, ProgressStage::Downloading, ProgressStage::Verifying, ProgressStage::Extracting, ProgressStage::Configuring]);
    let subjects: Vec<Option<String>> = progress.0.lock().unwrap().iter().map(|p| p.subject.clone()).collect();
    assert!(subjects.iter().all(|s| s.as_deref() == Some("php")), "every stage names the requested kind, so the UI can follow it: {subjects:?}");
    let downloads = k.downloader.downloads.lock().unwrap().clone();
    assert_eq!(downloads, [k.w.paths.downloads_dir().join("php-8.3.15-php-8.3.15.zip")]);
    assert_eq!(*k.downloader.discarded.lock().unwrap(), downloads, "the archive is deleted once extracted");
    assert_eq!(k.extractor.0.lock().unwrap()[0].1, std::path::PathBuf::from("C:\\lampo\\packages\\php-8.3.15"));
    assert_eq!(k.hook.calls.lock().unwrap().len(), 1, "the PHP hook ran");
    assert!(k.w.installed.has(&PackageKind::PHP, "8.3.15"));
    assert_eq!(k.w.settings.load().unwrap().default_php, Some(php("8.3")));
    assert!(k.w.events.all().contains(&DomainEvent::PackagesChanged));
    assert_eq!(k.sync.calls.load(Ordering::SeqCst), 1, "the shim index / config follow the new PHP");

    // A second PHP does not replace the default; the live source wins for "latest".
    let dto = k.install().execute(request("php", None, None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert_eq!(dto.id.version, "8.4.3");
    assert_eq!(k.w.settings.load().unwrap().default_php, Some(php("8.3")));

    // Already installed: nothing downloaded.
    let before = k.downloader.downloads.lock().unwrap().len();
    k.install().execute(request("php", Some("8.3.15"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert_eq!(k.downloader.downloads.lock().unwrap().len(), before);

    // Settings reset while packages stayed: asking for an installed PHP adopts it as default.
    let mut settings = k.w.settings.load().unwrap();
    settings.default_php = None;
    k.w.settings.save(&settings).unwrap();
    k.install().execute(request("php", Some("8.4.3"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert_eq!(k.w.settings.load().unwrap().default_php, Some(php("8.4")));
}

#[tokio::test]
async fn a_missing_vc_runtime_is_reported_before_downloading() {
    let k = Kit::new(World::new());
    k.vc.0.lock().unwrap().clear();
    let err = k.install().execute(request("php", Some("8.4.2"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PackageMissing);
    assert_eq!(err.details.unwrap()["downloadUrl"], "https://aka.ms/vs/17/release/vc_redist.x64.exe");
    assert!(k.downloader.downloads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unverifiable_or_corrupt_downloads_are_refused() {
    let k = Kit::new(World::new());
    k.source.entries.lock().unwrap().iter_mut().find(|e| e.id.kind == PackageKind::APACHE).unwrap().sha256 = None;
    let err = k.install().execute(request("apache", None, None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ChecksumMismatch);
    assert!(k.downloader.downloads.lock().unwrap().is_empty());

    *k.checksum.0.lock().unwrap() = digest(9);
    let err = k.install().execute(request("mysql", None, Some("8.4")), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ChecksumMismatch);
    assert!(k.extractor.0.lock().unwrap().is_empty(), "nothing extracted");
    assert_eq!(k.downloader.discarded.lock().unwrap().len(), 1, "the corrupt download is not kept for resume");
    assert!(k.w.installed.list().unwrap().is_empty(), "nothing recorded");
    assert_eq!(k.w.installed.removed.lock().unwrap().len(), 1, "partial files cleaned up");
}

#[tokio::test]
async fn installs_can_be_cancelled_mid_download() {
    let k = Kit::new(World::new());
    k.downloader.cancel_midway.store(true, Ordering::SeqCst);
    let err = k.install().execute(request("mysql", None, Some("8.4")), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert!(k.w.installed.list().unwrap().is_empty());
    assert_eq!(k.sync.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn single_file_packages_are_downloaded_into_place_and_not_extracted() {
    let k = Kit::new(World::new());
    k.install().execute(request("composer", None, None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert_eq!(k.downloader.downloads.lock().unwrap()[0], std::path::PathBuf::from("C:\\lampo\\packages\\composer-2.8.4\\composer.phar"));
    assert!(k.extractor.0.lock().unwrap().is_empty());
    assert!(k.downloader.discarded.lock().unwrap().is_empty(), "the downloaded file is the package");
}

#[test]
fn php_extensions_come_from_the_installed_build_of_that_minor() {
    struct Scanner(Mutex<Vec<std::path::PathBuf>>);
    impl lampo_domain::ports::PhpExtensionScanner for Scanner {
        fn bundled_extensions(&self, php_dir: &std::path::Path) -> lampo_domain::PortResult<Vec<lampo_domain::php::ExtensionName>> {
            self.0.lock().unwrap().push(php_dir.to_path_buf());
            Ok(["intl", "opcache"].iter().map(|n| lampo_domain::php::ExtensionName::new(n).unwrap()).collect())
        }
    }
    let w = World::with_basics();
    let scanner = Arc::new(Scanner(Mutex::new(Vec::new())));
    let query = PhpExtensionsQuery::new(w.installed.clone(), scanner.clone());
    let list = query.list("8.3").unwrap();
    assert_eq!(list, [PhpExtensionDto { name: "intl".into(), zend: false }, PhpExtensionDto { name: "opcache".into(), zend: true }]);
    let php83 = w.installed.list().unwrap().into_iter().find(|p| p.id.kind == PackageKind::PHP && p.id.series().to_string() == "8.3").unwrap();
    assert_eq!(scanner.0.lock().unwrap()[0], php83.install_dir);
    assert_eq!(query.list("5.6").unwrap_err().code, ErrorCode::PackageMissing);
    assert_eq!(query.list("nope").unwrap_err().code, ErrorCode::Validation);
}

/// `intl` and `opcache` in every PHP build.
struct TwoExtensions;
impl lampo_domain::ports::PhpExtensionScanner for TwoExtensions {
    fn bundled_extensions(&self, _: &std::path::Path) -> lampo_domain::PortResult<Vec<lampo_domain::php::ExtensionName>> {
        Ok(["intl", "opcache"].iter().map(|n| lampo_domain::php::ExtensionName::new(n).unwrap()).collect())
    }
}

fn listing(w: &World) -> PhpExtensionListing {
    PhpExtensionListing::new(w.resolver(), w.projects.clone(), Arc::new(PhpExtensionsQuery::new(w.installed.clone(), Arc::new(TwoExtensions))))
}

fn at(cwd: &str) -> ResolveRequestDto {
    ResolveRequestDto { cwd: cwd.into(), explicit_site: None }
}

#[test]
fn extension_listing_uses_the_resolved_project_php_and_flags_what_it_enables() {
    let w = World::with_basics();
    let mut shop = w.project("shop", "C:\\sites\\shop", false);
    shop.environment.extensions.insert(lampo_domain::php::ExtensionName::new("intl").unwrap());
    w.projects.save(&shop).unwrap();
    w.env.write_generated_files().unwrap();
    let enabled = |l: &PhpExtensionListDto| l.extensions.iter().map(|e| (e.name.clone(), e.enabled)).collect::<Vec<_>>();

    let list = listing(&w).execute(&at("C:\\sites\\shop\\app"), None).unwrap();
    assert_eq!((list.php_version.as_str(), list.project_id.as_deref()), ("8.3", Some("shop")));
    assert_eq!(enabled(&list), [("intl".to_string(), Some(true)), ("opcache".to_string(), Some(false))]);
    assert!(list.extensions[1].zend);

    // Another minor: listed, but the project's flags do not apply to it.
    let other = listing(&w).execute(&at("C:\\sites\\shop"), Some("8.4")).unwrap();
    assert_eq!((other.php_version.as_str(), other.project_id), ("8.4", None));
    assert!(other.extensions.iter().all(|e| e.enabled.is_none()));
    // Explicit and matching: flags again.
    assert_eq!(listing(&w).execute(&at("C:\\sites\\shop"), Some("8.3")).unwrap().project_id.as_deref(), Some("shop"));

    // Outside any project: the global default PHP (8.4), no flags.
    let outside = listing(&w).execute(&at("D:\\elsewhere"), None).unwrap();
    assert_eq!((outside.php_version.as_str(), outside.project_id), ("8.4", None));

    assert_eq!(listing(&w).execute(&at("D:\\elsewhere"), Some("5.6")).unwrap_err().code, ErrorCode::PackageMissing);
    assert_eq!(listing(&w).execute(&at("D:\\elsewhere"), Some("x")).unwrap_err().code, ErrorCode::Validation);
}

#[test]
fn extension_listing_without_a_site_or_default_php_needs_an_explicit_minor() {
    // No shim index written yet: no site and no default target.
    let w = World::with_basics();
    let err = listing(&w).execute(&at("D:\\elsewhere"), None).unwrap_err();
    assert_eq!(err.code, ErrorCode::ProjectNotResolved);
    assert!(err.message.contains("--php"), "{}", err.message);
    assert_eq!(listing(&w).execute(&at("D:\\elsewhere"), Some("8.3")).unwrap().extensions.len(), 2);
}

fn uninstall_request(kind: &str, version: Option<&str>, series: Option<&str>, php: Option<&str>) -> UninstallPackageRequest {
    UninstallPackageRequest { kind: kind.into(), version: version.map(Into::into), series: series.map(Into::into), php_version: php.map(Into::into) }
}

#[test]
fn uninstall_by_kind_version_and_php_picks_exactly_one_build() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.installed.add(PackageKind::XDEBUG, "3.3.2", Some("8.3"));
    w.installed.add(PackageKind::XDEBUG, "3.3.2", Some("8.4"));
    let uninstall = w.uninstall_package();

    // Two builds match: nothing is removed, the candidates are listed.
    let err = uninstall.execute_matching(uninstall_request("xdebug", None, None, None)).unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.message.contains("--php"), "{}", err.message);
    assert_eq!(err.details.unwrap()["candidates"].as_array().unwrap().len(), 2);
    let removed = uninstall.execute_matching(uninstall_request("xdebug", None, Some("3.3"), Some("8.4"))).unwrap();
    assert_eq!(removed, PackageIdDto { kind: "xdebug".into(), version: "3.3.2".into(), php_version: Some("8.4".into()) });
    assert_eq!(w.installed.list().unwrap().iter().filter(|p| p.id.kind == PackageKind::XDEBUG).count(), 1);

    // Series and exact version; nothing matching is NotFound (exit 3), never a guess.
    let mysql = uninstall.execute_matching(uninstall_request("mysql", None, Some("5.7"), None)).unwrap();
    assert_eq!(mysql.version, "5.7.44");
    let none = uninstall.execute_matching(uninstall_request("mysql", None, Some("5.7"), None)).unwrap_err();
    assert_eq!(none.code, ErrorCode::NotFound);
    assert!(none.message.contains("lampo package list --installed"), "{}", none.message);
    assert_eq!(uninstall.execute_matching(uninstall_request("php", Some("8.3.99"), None, None)).unwrap_err().code, ErrorCode::NotFound);

    // In use: Conflict with who uses it and what to change; the build stays.
    let err = uninstall.execute_matching(uninstall_request("php", Some("8.3.14"), None, None)).unwrap_err();
    assert_eq!((err.code, err.details.clone().unwrap()["usedBy"].clone()), (ErrorCode::Conflict, serde_json::json!(["shop"])));
    assert!(err.message.contains("in use by shop") && err.message.contains("lampo project set-php"), "{}", err.message);
    assert!(w.installed.has(&PackageKind::PHP, "8.3.14"));
    assert_eq!(uninstall.execute_matching(uninstall_request("php", None, Some("8.4"), None)).unwrap_err().code, ErrorCode::Conflict, "the default PHP");

    assert_eq!(uninstall.execute_matching(uninstall_request("not a kind!", None, None, None)).unwrap_err().code, ErrorCode::Validation);
}

#[tokio::test]
async fn a_failing_live_catalog_degrades_to_the_others() {
    let k = Kit::new(World::new());
    k.live.fail.store(true, Ordering::SeqCst);
    let dto = k.install().execute(request("php", None, Some("8.4")), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert_eq!(dto.id.version, "8.4.2");
    let listed = k.catalog().list(Some("php")).await.unwrap();
    assert_eq!(listed.len(), 4);
    let p84 = listed.iter().find(|e| e.id.version == "8.4.2").unwrap();
    assert_eq!((p84.vc_runtime.as_deref(), p84.vc_runtime_installed, p84.installed), (Some("vs17"), Some(true), true));

    k.source.fail.store(true, Ordering::SeqCst);
    assert_eq!(k.catalog().list(Some("php")).await.unwrap_err().code, ErrorCode::Network);
    let unknown = k.install().execute(request("php", Some("9.9.9"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(unknown.code, ErrorCode::Network);
}

#[tokio::test]
async fn updates_and_usage_are_reported_per_series() {
    let k = Kit::new(World::with_basics());
    k.w.project("shop", "C:\\sites\\shop", true);
    let updates = k.catalog().check_updates().await.unwrap();
    let php = updates.iter().filter(|u| u.kind == "php").map(|u| (u.series.as_str(), u.latest.as_str())).collect::<Vec<_>>();
    assert_eq!(php, [("8.3", "8.3.15"), ("8.4", "8.4.3")]);
    let installed = k.catalog().list_installed().unwrap();
    let used = |kind: &str, version: &str| installed.iter().find(|p| p.id.kind == kind && p.id.version == version).unwrap().used_by.clone();
    assert_eq!(used("php", "8.3.14"), ["shop"]);
    assert_eq!(used("php", "8.4.1"), ["system"]);
    assert_eq!(used("mysql", "8.4.3"), ["shop", "system"]);
    assert_eq!(used("apache", "2.4.62"), ["shop"]);
    assert!(used("mysql", "5.7.44").is_empty());
}

#[tokio::test]
async fn uninstall_refuses_the_last_build_in_use() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    let uninstall = w.uninstall_package();
    let php83 = PackageIdDto { kind: "php".into(), version: "8.3.14".into(), php_version: None };
    let err = uninstall.execute(php83.clone()).unwrap_err();
    assert_eq!((err.code, err.details.unwrap()["usedBy"].clone()), (ErrorCode::Conflict, serde_json::json!(["shop"])));

    // Another 8.3 build satisfies the project: the old one can go.
    let newer = w.installed.add(PackageKind::PHP, "8.3.15", None);
    uninstall.execute(php83).unwrap();
    assert!(!w.installed.has(&PackageKind::PHP, "8.3.14"));
    assert!(w.events.all().contains(&DomainEvent::PackagesChanged));
    assert_eq!(uninstall.execute(package_id_to_dto(&newer.id)).unwrap_err().code, ErrorCode::Conflict);

    // The always-on MySQL and the default PHP are protected too; unused packages are not.
    assert_eq!(uninstall.execute(PackageIdDto { kind: "mysql".into(), version: "8.4.3".into(), php_version: None }).unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(uninstall.execute(PackageIdDto { kind: "php".into(), version: "8.4.1".into(), php_version: None }).unwrap_err().code, ErrorCode::Conflict);
    uninstall.execute(PackageIdDto { kind: "mysql".into(), version: "5.7.44".into(), php_version: None }).unwrap();
    assert_eq!(uninstall.execute(PackageIdDto { kind: "redis".into(), version: "1.0.0".into(), php_version: None }).unwrap_err().code, ErrorCode::NotFound);
}

#[tokio::test]
async fn first_run_installs_what_is_missing_and_is_resumable() {
    let k = Kit::new(World::new());
    k.w.installed.add(PackageKind::APACHE, "2.4.62", None);
    let shims = k.w.shim_setup();
    let setup = k.first_run(shims.clone());

    let status = setup.status().unwrap();
    assert!(!status.completed);
    assert_eq!(
        status.items.iter().map(|i| (i.kind.as_str(), i.series.as_deref(), i.installed)).collect::<Vec<_>>(),
        [("apache", None, true), ("mod-fcgid", None, false), ("php", None, false), ("mysql", Some("8.4"), false), ("composer", None, false)]
    );

    let progress = RecordingProgress::default();
    let done = setup.run(&cli(), &progress, &CancelFlag::new()).await.unwrap();
    assert!(done.completed);
    assert!(done.items.iter().all(|i| i.installed));
    assert!(k.w.installed.has(&PackageKind::PHP, "8.4.3"), "latest PHP");
    assert!(k.w.installed.has(&PackageKind::MYSQL, "8.4.3"));
    assert!(!k.w.installed.has(&PackageKind::MYSQL, "5.7.44"));
    assert!(shims.status().unwrap().installed && shims.status().unwrap().on_path);
    assert_eq!(k.w.settings.load().unwrap().default_php, Some(php("8.4")));
    let downloads = k.downloader.downloads.lock().unwrap().len();
    assert_eq!(downloads, 4);

    // Resumable / idempotent: nothing left to install.
    setup.run(&cli(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert_eq!(k.downloader.downloads.lock().unwrap().len(), downloads);

    // An old PHP makes Composer 2.2 LTS part of the setup.
    k.w.installed.add(PackageKind::PHP, "5.6.40", None);
    let status = setup.status().unwrap();
    assert_eq!(status.items.last().map(|i| (i.kind.as_str(), i.series.as_deref(), i.installed)), Some(("composer", Some("2.2"), false)));
}

#[tokio::test]
async fn first_run_can_be_cancelled_between_items() {
    let k = Kit::new(World::new());
    let setup = k.first_run(k.w.shim_setup());
    let cancel = CancelFlag::new();
    cancel.cancel();
    assert_eq!(setup.run(&cli(), &RecordingProgress::default(), &cancel).await.unwrap_err().code, ErrorCode::Cancelled);
    assert!(!k.w.settings.load().unwrap().first_run_completed);
}

#[tokio::test]
async fn startup_waits_for_first_run_and_shutdown_stops_everything() {
    let w = World::with_basics();
    w.hosts_in_sync();
    w.settings.0.lock().unwrap().first_run_completed = false;
    Startup::new(w.env.clone(), w.settings.clone(), w.shim_setup(), Arc::new(FakeAppLocation::default())).run().await.unwrap();
    assert!(w.all_calls().is_empty());
    w.settings.0.lock().unwrap().first_run_completed = true;
    Startup::new(w.env.clone(), w.settings.clone(), w.shim_setup(), Arc::new(FakeAppLocation::default())).run().await.unwrap();
    assert_eq!(w.mysql.calls(), ["prepare:mysql-8.4", "start:mysql-8.4"]);

    Shutdown::new(w.runners(), w.services.clone(), w.settings.clone()).run().await.unwrap();
    assert_eq!(w.mysql.calls().last().unwrap(), "stop:mysql-8.4");
}

/// After an app update the shim copies are the old `lampo-shim.exe`: startup refreshes them,
/// before (and regardless of) the first-run check, and never installs shims that were not there.
#[tokio::test]
async fn startup_refreshes_installed_shims() {
    let w = World::with_basics();
    w.hosts_in_sync();
    let installer = Arc::new(FakeShimInstaller::default());
    let shims = Arc::new(ShimSetup::new(installer.clone(), Arc::new(FakeUserPath::default()), Arc::new(FakeLocator::default()), w.paths.clone(), lampo_domain::shim::tools::all()));
    let location = Arc::new(FakeAppLocation::default());
    let startup = Startup::new(w.env.clone(), w.settings.clone(), shims, location.clone());
    startup.run().await.unwrap();
    assert_eq!(location.0.load(Ordering::SeqCst), 1, "the app location is recorded at every start");
    assert_eq!(installer.1.load(Ordering::SeqCst), 0, "never installed: nothing to refresh");
    assert!(!installer.0.load(Ordering::SeqCst));

    installer.0.store(true, Ordering::SeqCst);
    w.settings.0.lock().unwrap().first_run_completed = false;
    startup.run().await.unwrap();
    assert_eq!(installer.1.load(Ordering::SeqCst), 1);
    assert_eq!(location.0.load(Ordering::SeqCst), 2, "before the first-run check too");
}

/// M1: shutdown is driven by the settings (a required dependency), so a MySQL series other than
/// the two defaults is stopped cleanly instead of being killed by the Job Object.
#[tokio::test]
async fn shutdown_stops_custom_mysql_series_too() {
    use lampo_domain::mysql::MysqlSeries;
    use lampo_domain::service::{ServiceId, ServiceState};
    use lampo_domain::settings::ServiceInstanceSettings;
    let w = World::with_basics();
    let custom = MysqlSeries::parse("8.0").unwrap();
    w.settings.0.lock().unwrap().services.push(ServiceInstanceSettings::new(custom.service_id(), 3308, false));
    w.mysql.set(&custom.service_id(), ServiceState::Running);
    w.apache.set(&ServiceId::apache(), ServiceState::Running);
    Shutdown::new(w.runners(), w.services.clone(), w.settings.clone()).run().await.unwrap();
    assert_eq!(w.mysql.calls(), ["stop:mysql-8.0"]);
    assert_eq!(w.apache.calls(), ["stop:apache"], "Apache first");
}

/// M1: the first-run wizard always installs the shims (no optional builder to forget).
#[tokio::test]
async fn first_run_always_ends_with_the_shims() {
    let k = Kit::new(World::with_basics());
    let shims = k.w.shim_setup();
    k.first_run(shims.clone()).run(&cli(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert!(shims.status().unwrap().installed);
}
