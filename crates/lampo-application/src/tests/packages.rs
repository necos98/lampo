//! Package catalog, install pipeline, uninstall rules, first run.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use lampo_domain::events::DomainEvent;
use lampo_domain::package::{PackageKind, PackagePayload};
use lampo_domain::ports::{InstalledPackageRegistry, PackageCatalogSource, ProjectRepository, SettingsRepository};
use lampo_domain::progress::{CancelFlag, ProgressStage};
use lampo_domain::vc_runtime::VcRuntime;
use lampo_domain::tcp_port::Port;

use super::allocation::after_a_locked_commit;
use super::{cli, gui};
use crate::dto::*;
use crate::error::{AppResult, ErrorCode};
use crate::mapping::package_id_to_dto;
use crate::testing::*;
use crate::use_cases::*;

struct Kit {
    w: World,
    source: Arc<FakeSource>,
    live: Arc<FakeSource>,
    downloader: Arc<FakeDownloader>,
    extractor: Arc<FakeExtractor>,
    hook: Arc<FakeHook>,
    slots: Arc<MemSlotLock>,
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
            extractor: Arc::new(FakeExtractor::default()),
            hook: Arc::new(FakeHook::new(PackageKind::PHP)),
            slots: Arc::new(MemSlotLock::default()),
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
            extractor: self.extractor.clone(),
            installed: self.w.installed.clone(),
            slots: self.slots.clone(),
            hooks: vec![self.hook.clone()],
            vc: self.vc.clone(),
            settings: self.w.settings_writer(),
            paths: self.w.paths.clone(),
            env: self.sync.clone(),
            events: self.w.events.clone(),
        }))
    }

    fn first_run(&self, shims: Arc<ShimSetup>) -> FirstRunSetup {
        FirstRunSetup::new(FirstRunDeps {
            settings: self.w.settings_writer(),
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
    assert_eq!(k.extractor.extracted.lock().unwrap()[0].1, std::path::PathBuf::from("C:\\lampo\\packages\\php-8.3.15"));
    assert_eq!(k.hook.calls.lock().unwrap().len(), 1, "the PHP hook ran");
    assert!(k.w.installed.has(&PackageKind::PHP, "8.3.15"));
    let committed: Vec<_> = k.extractor.committed.lock().unwrap().iter().map(|r| r.destination.clone()).collect();
    assert_eq!(committed, [std::path::PathBuf::from("C:\\lampo\\packages\\php-8.3.15")], "the previous version is deleted once installed");
    assert!(k.extractor.rolled_back.lock().unwrap().is_empty());
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

/// A settings save made by another commit under the allocator lock: Apache moves to 8080.
fn apache_moved_to_8080(w: &World) -> impl FnOnce(&SiteSnapshot) -> AppResult<()> + Send + 'static {
    let repo = w.settings.clone();
    move |snap| {
        let mut next = snap.settings.clone();
        next.apache_port = Port::of(8080);
        Ok(repo.save(&next)?)
    }
}

/// Adopting the first PHP as the default is best effort: a failed settings save does not fail an
/// installation that is already complete (still recorded, announced and synced).
#[tokio::test]
async fn a_failed_default_php_adoption_does_not_fail_the_install() {
    let k = Kit::new(World::new());
    k.w.settings.fail_saves(true);
    let dto = k.install().execute(request("php", None, Some("8.3")), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert_eq!(dto.id.version, "8.3.15");
    assert!(k.w.installed.has(&PackageKind::PHP, "8.3.15"));
    assert!(k.w.events.all().contains(&DomainEvent::PackagesChanged));
    assert_eq!(k.sync.calls.load(Ordering::SeqCst), 1, "synced after the failed adoption");
    assert_eq!(k.w.settings.load().unwrap().default_php, None);

    // Already installed: the adoption is retried, and its failure is still not the install's.
    let again = k.install().execute(request("php", Some("8.3.15"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert_eq!(again.id, dto.id);
}

/// The default-PHP adoption saves under the allocator lock, built on the settings stored then: a
/// concurrent save is kept, not reverted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_default_php_adoption_waits_for_the_allocator_lock() {
    let k = Kit::new(World::new());
    let install = k.install();
    let op = async move { install.execute(request("php", None, Some("8.3")), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await };
    after_a_locked_commit(&k.w, apache_moved_to_8080(&k.w), op).await.unwrap();
    let stored = k.w.settings.load().unwrap();
    assert_eq!((stored.apache_port, stored.default_php), (Port::of(8080), Some(php("8.3"))));
}

/// The wizard records its completion under the allocator lock (here it has nothing left to
/// install, so that save is the only one that needs the lock).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_run_completion_waits_for_the_allocator_lock() {
    let k = Kit::new(World::new());
    for (kind, version) in [(PackageKind::APACHE, "2.4.62"), (PackageKind::MOD_FCGID, "2.3.10"), (PackageKind::PHP, "8.4.3"), (PackageKind::MYSQL, "8.4.3"), (PackageKind::COMPOSER, "2.8.4")]
    {
        k.w.installed.add(kind, version, None);
    }
    let setup = k.first_run(k.w.shim_setup());
    let op = async move { setup.run(&cli(), &RecordingProgress::default(), &CancelFlag::new()).await };
    let status = after_a_locked_commit(&k.w, apache_moved_to_8080(&k.w), op).await.unwrap();
    assert!(status.completed);
    let stored = k.w.settings.load().unwrap();
    assert!(stored.first_run_completed);
    assert_eq!(stored.apache_port, 8080, "the concurrent save is kept");
}

#[tokio::test]
async fn a_missing_vc_runtime_is_reported_before_downloading() {
    let k = Kit::new(World::new());
    k.vc.0.lock().unwrap().clear();
    let err = k.install().execute(request("php", Some("8.4.2"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PackageMissing);
    let details = err.details.unwrap();
    assert_eq!(details["downloadUrl"], "https://aka.ms/vs/17/release/vc_redist.x64.exe");
    assert_eq!(details["package"], "php-8.4.2", "the GUI names the package that needs the runtime");
    assert!(k.downloader.downloads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn concurrent_installs_of_one_package_download_it_once() {
    let k = Kit::new(World::new());
    k.downloader.slow.store(true, Ordering::SeqCst);
    let install = k.install();
    let (ctx, cancel) = (gui(), CancelFlag::new());
    let run = || install.execute(request("php", Some("8.3.15"), None), &ctx, &lampo_domain::ports::NullProgressSink, &cancel);
    let (first, second) = tokio::join!(run(), run());
    assert_eq!(first.unwrap().id, second.unwrap().id);
    assert_eq!(k.downloader.peak_running.load(Ordering::SeqCst), 1);
    assert_eq!(k.downloader.downloads.lock().unwrap().len(), 1, "the second request waits for the first and finds the package installed");
    assert_eq!(k.extractor.extracted.lock().unwrap().len(), 1, "one extraction into the slot");
}

#[tokio::test]
async fn installs_into_different_slots_run_side_by_side() {
    let k = Kit::new(World::new());
    k.downloader.slow.store(true, Ordering::SeqCst);
    let install = k.install();
    let (ctx, cancel) = (gui(), CancelFlag::new());
    let run = |kind: &'static str, series: &'static str| install.execute(request(kind, None, Some(series)), &ctx, &lampo_domain::ports::NullProgressSink, &cancel);
    let (php, mysql) = tokio::join!(run("php", "8.3"), run("mysql", "8.4"));
    php.unwrap();
    mysql.unwrap();
    assert_eq!(k.downloader.peak_running.load(Ordering::SeqCst), 2, "the two downloads overlapped");
}

/// A failing post-install hook must not cost the version installed before: the slot gets it
/// back, nothing new is recorded and the previous copy is not deleted.
#[tokio::test]
async fn a_failing_hook_restores_the_previous_version() {
    let k = Kit::new(World::new());
    let previous = k.w.installed.add(PackageKind::PHP, "8.3.14", None);
    k.hook.fails.store(true, Ordering::SeqCst);
    let err = k.install().execute(request("php", Some("8.3.15"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Io, "{err:?}");
    let rolled_back = k.extractor.rolled_back.lock().unwrap().clone();
    assert_eq!(rolled_back.iter().map(|r| r.destination.clone()).collect::<Vec<_>>(), [std::path::PathBuf::from("C:\\lampo\\packages\\php-8.3.15")]);
    assert!(k.extractor.committed.lock().unwrap().is_empty(), "the previous version is kept");
    assert_eq!(k.w.installed.list().unwrap().iter().map(|p| p.id.clone()).collect::<Vec<_>>(), [previous.id], "only the previous version is recorded");
    assert!(k.w.installed.removed.lock().unwrap().is_empty(), "no record is removed: the failed one was never written");
    assert!(!k.w.events.all().contains(&DomainEvent::PackagesChanged));
}

/// The slot lock is released on failure: a retry is not blocked by the failed attempt.
#[tokio::test]
async fn a_retry_after_a_failed_install_proceeds() {
    let k = Kit::new(World::new());
    k.hook.fails.store(true, Ordering::SeqCst);
    let install = k.install();
    install.execute(request("php", Some("8.3.15"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap_err();
    k.hook.fails.store(false, Ordering::SeqCst);
    install.execute(request("php", Some("8.3.15"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert!(k.w.installed.has(&PackageKind::PHP, "8.3.15"));
    assert_eq!(k.extractor.committed.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn unverifiable_or_corrupt_downloads_are_refused() {
    let k = Kit::new(World::new());
    k.source.entries.lock().unwrap().iter_mut().find(|e| e.id.kind == PackageKind::APACHE).unwrap().sha256 = None;
    let err = k.install().execute(request("apache", None, None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ChecksumMismatch);
    assert!(k.downloader.downloads.lock().unwrap().is_empty());

    *k.downloader.digest.lock().unwrap() = digest(9);
    let err = k.install().execute(request("mysql", None, Some("8.4")), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ChecksumMismatch);
    assert!(k.extractor.extracted.lock().unwrap().is_empty(), "nothing extracted");
    assert_eq!(k.downloader.discarded.lock().unwrap().len(), 1, "the corrupt download is not kept for resume");
    assert!(k.w.installed.list().unwrap().is_empty(), "nothing recorded");
    assert!(k.extractor.rolled_back.lock().unwrap().is_empty() && k.extractor.committed.lock().unwrap().is_empty(), "the slot was never touched");
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
async fn single_file_packages_are_downloaded_aside_verified_then_placed() {
    let k = Kit::new(World::new());
    k.install().execute(request("composer", None, None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    let download = k.w.paths.downloads_dir().join("composer-2.8.4-composer-2.8.4.zip");
    assert_eq!(*k.downloader.downloads.lock().unwrap(), std::slice::from_ref(&download), "never straight into the install dir");
    assert_eq!(*k.extractor.placed.lock().unwrap(), [(download, std::path::PathBuf::from("C:\\lampo\\packages\\composer-2.8.4"), "composer.phar".to_string())]);
    assert!(k.extractor.extracted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_corrupt_single_file_download_leaves_the_installed_version_alone() {
    let k = Kit::new(World::new());
    k.install().execute(request("composer", Some("2.8.4"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    let mut patch = catalog_entry(PackageKind::COMPOSER, "2.8.5", None);
    patch.payload = PackagePayload::SingleFile { file_name: "composer.phar".into() };
    k.source.entries.lock().unwrap().push(patch);
    *k.downloader.digest.lock().unwrap() = digest(9);

    let err = k.install().execute(request("composer", Some("2.8.5"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ChecksumMismatch);
    assert_eq!(k.extractor.placed.lock().unwrap().len(), 1, "only the first, verified version was placed");
    assert!(k.downloader.downloads.lock().unwrap().iter().all(|d| d.starts_with(k.w.paths.downloads_dir())));
    assert!(k.w.installed.has(&PackageKind::COMPOSER, "2.8.4"), "the previous version is still installed");
}

#[tokio::test]
async fn downloads_are_limited_to_the_catalog_size_plus_the_tolerance() {
    let k = Kit::new(World::new());
    k.install().execute(request("mysql", None, Some("8.4")), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert_eq!(*k.downloader.limits.lock().unwrap(), [Some(1100)], "catalog_entry sizes are 1000 bytes");
}

#[tokio::test]
async fn a_live_hash_contradicting_the_pinned_one_is_refused_before_downloading() {
    let k = Kit::new(World::new());
    let mut tampered = catalog_entry(PackageKind::PHP, "8.3.15", None);
    tampered.channel = lampo_domain::package::CatalogChannel::Live;
    tampered.sha256 = Some(digest(9));
    k.live.entries.lock().unwrap().push(tampered);

    let err = k.install().execute(request("php", Some("8.3.15"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ChecksumMismatch);
    assert!(err.message.contains("integrity"), "{}", err.message);
    assert!(k.downloader.downloads.lock().unwrap().is_empty());
    let listed = k.catalog().list(Some("php")).await.unwrap();
    assert!(listed.iter().any(|e| e.id.version == "8.3.15"), "the catalog still lists it");
    k.install().execute(request("php", Some("8.3.14"), None), &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
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
    let missing = query.list("5.6").unwrap_err();
    assert_eq!((missing.code, missing.details), (ErrorCode::PackageMissing, Some(serde_json::json!({ "package": "PHP 5.6" }))));
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
    ResolveRequestDto { cwd: cwd.into(), ..Default::default() }
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

#[tokio::test]
async fn uninstall_by_kind_version_and_php_picks_exactly_one_build() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.installed.add(PackageKind::XDEBUG, "3.3.2", Some("8.3"));
    w.installed.add(PackageKind::XDEBUG, "3.3.2", Some("8.4"));
    let uninstall = w.uninstall_package();

    // Two builds match: nothing is removed, the candidates are listed.
    let err = uninstall.execute_matching(uninstall_request("xdebug", None, None, None), &gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.message.contains("--php"), "{}", err.message);
    assert_eq!(err.details.unwrap()["candidates"].as_array().unwrap().len(), 2);
    let removed = uninstall.execute_matching(uninstall_request("xdebug", None, Some("3.3"), Some("8.4")), &gui()).await.unwrap();
    assert_eq!(removed, PackageIdDto { kind: "xdebug".into(), version: "3.3.2".into(), php_version: Some("8.4".into()) });
    assert_eq!(w.installed.list().unwrap().iter().filter(|p| p.id.kind == PackageKind::XDEBUG).count(), 1);

    // Series and exact version; nothing matching is NotFound (exit 3), never a guess.
    let mysql = uninstall.execute_matching(uninstall_request("mysql", None, Some("5.7"), None), &gui()).await.unwrap();
    assert_eq!(mysql.version, "5.7.44");
    let none = uninstall.execute_matching(uninstall_request("mysql", None, Some("5.7"), None), &gui()).await.unwrap_err();
    assert_eq!(none.code, ErrorCode::NotFound);
    assert!(none.message.contains("lampo package list --installed"), "{}", none.message);
    assert_eq!(uninstall.execute_matching(uninstall_request("php", Some("8.3.99"), None, None), &gui()).await.unwrap_err().code, ErrorCode::NotFound);

    // In use: Conflict with who uses it and what to change; the build stays.
    let err = uninstall.execute_matching(uninstall_request("php", Some("8.3.14"), None, None), &gui()).await.unwrap_err();
    assert_eq!((err.code, err.details.clone().unwrap()["usedBy"].clone()), (ErrorCode::Conflict, serde_json::json!(["shop"])));
    assert_eq!(err.details.clone().unwrap()["package"], "php-8.3.14", "the GUI names the package in use");
    assert!(err.message.contains("in use by shop") && err.message.contains("lampo project set-php"), "{}", err.message);
    assert!(w.installed.has(&PackageKind::PHP, "8.3.14"));
    assert_eq!(uninstall.execute_matching(uninstall_request("php", None, Some("8.4"), None), &gui()).await.unwrap_err().code, ErrorCode::Conflict, "the default PHP");

    assert_eq!(uninstall.execute_matching(uninstall_request("not a kind!", None, None, None), &gui()).await.unwrap_err().code, ErrorCode::Validation);
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

/// A source that answers only once every source of the test has been asked: listings that
/// queried the sources one after the other would never finish.
struct RendezvousSource {
    inner: Arc<FakeSource>,
    all_asked: Arc<tokio::sync::Barrier>,
}

#[async_trait::async_trait]
impl PackageCatalogSource for RendezvousSource {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn kinds(&self) -> Vec<PackageKind> {
        self.inner.kinds()
    }
    async fn fetch(&self, kind: &PackageKind) -> lampo_domain::PortResult<Vec<lampo_domain::package::CatalogEntry>> {
        self.all_asked.wait().await;
        self.inner.fetch(kind).await
    }
}

#[tokio::test]
async fn catalog_sources_are_queried_concurrently_and_merged_in_order() {
    let mut live = catalog_entry(PackageKind::PHP, "8.4.2", None);
    live.url = lampo_domain::package::DownloadUrl::new("https://example.invalid/moved.zip").unwrap();
    let pinned = catalog_entry(PackageKind::PHP, "8.4.2", None);
    let all_asked = Arc::new(tokio::sync::Barrier::new(2));
    let sources: Vec<Arc<dyn PackageCatalogSource>> = [FakeSource::new("live", vec![live]), FakeSource::new("embedded", vec![pinned])]
        .into_iter()
        .map(|inner| Arc::new(RendezvousSource { inner, all_asked: all_asked.clone() }) as Arc<dyn PackageCatalogSource>)
        .collect();
    let catalog = tokio::time::timeout(std::time::Duration::from_secs(5), packages::fetch_catalog(&sources, Some(&PackageKind::PHP))).await.expect("sources queried concurrently").unwrap();
    assert_eq!(catalog.entries.len(), 1);
    assert_eq!(catalog.entries[0].url.as_str(), "https://example.invalid/moved.zip", "the first source still wins on duplicates");
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
    let err = uninstall.execute(php83.clone(), &gui()).await.unwrap_err();
    assert_eq!((err.code, err.details.unwrap()["usedBy"].clone()), (ErrorCode::Conflict, serde_json::json!(["shop"])));

    // Another 8.3 build satisfies the project: the old one can go.
    let newer = w.installed.add(PackageKind::PHP, "8.3.15", None);
    uninstall.execute(php83, &gui()).await.unwrap();
    assert!(!w.installed.has(&PackageKind::PHP, "8.3.14"));
    assert!(w.events.all().contains(&DomainEvent::PackagesChanged));
    assert_eq!(uninstall.execute(package_id_to_dto(&newer.id), &gui()).await.unwrap_err().code, ErrorCode::Conflict);

    // The always-on MySQL and the default PHP are protected too; unused packages are not.
    assert_eq!(uninstall.execute(PackageIdDto { kind: "mysql".into(), version: "8.4.3".into(), php_version: None }, &gui()).await.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(uninstall.execute(PackageIdDto { kind: "php".into(), version: "8.4.1".into(), php_version: None }, &gui()).await.unwrap_err().code, ErrorCode::Conflict);
    uninstall.execute(PackageIdDto { kind: "mysql".into(), version: "5.7.44".into(), php_version: None }, &gui()).await.unwrap();
    assert_eq!(uninstall.execute(PackageIdDto { kind: "redis".into(), version: "1.0.0".into(), php_version: None }, &gui()).await.unwrap_err().code, ErrorCode::NotFound);
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

/// Records, at each sweep, how many calls the MySQL runner had received.
struct SweepProbe {
    mysql: Arc<FakeRunner>,
    seen: Mutex<Vec<usize>>,
}
#[async_trait::async_trait]
impl lampo_domain::ports::PackageLeftoverSweeper for SweepProbe {
    async fn sweep(&self) {
        self.seen.lock().unwrap().push(self.mysql.calls().len());
    }
}

/// A slot a crash left mid-update must be repaired before a service starts from it.
#[tokio::test]
async fn startup_sweeps_package_leftovers_before_starting_services() {
    let w = World::with_basics();
    w.hosts_in_sync();
    let probe = Arc::new(SweepProbe { mysql: w.mysql.clone(), seen: Mutex::default() });
    Startup::new(w.env.clone(), w.settings.clone(), w.shim_setup(), Arc::new(FakeAppLocation::default())).with_package_sweeper(probe.clone()).run().await.unwrap();
    assert_eq!(*probe.seen.lock().unwrap(), [0], "one sweep, before MySQL was touched");
    assert_eq!(w.mysql.calls(), ["prepare:mysql-8.4", "start:mysql-8.4"]);
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
    w.settings.0.lock().unwrap().services.push(ServiceInstanceSettings::new(custom.service_id(), Port::of(3308), false));
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
