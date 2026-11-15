//! Uninstall moves what uses a package to the build that remains before its files go.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lampo_domain::package::{InstalledPackage, PackageId, PackageKind};
use lampo_domain::ports::InstalledPackageRegistry;
use lampo_domain::{PortError, PortResult};

use super::gui;
use crate::dto::PackageIdDto;
use crate::error::ErrorCode;
use crate::testing::{FakeWeb, MemInstalled, World};
use crate::use_cases::EnvironmentSync;

const PHP_8_3_14_CGI: &str = "C:\\lampo\\packages\\php-8.3.14\\php-cgi.exe";
const PHP_8_3_15_CGI: &str = "C:\\lampo\\packages\\php-8.3.15\\php-cgi.exe";

/// The world's registry, recording which `php-cgi.exe` Apache served when the files were deleted
/// (the moment a Windows lock or a dangling vhost would bite). Optionally refuses the removal,
/// like a locked folder.
struct ObservedRemoval {
    inner: Arc<MemInstalled>,
    web: Arc<FakeWeb>,
    served_at_removal: Mutex<Option<PathBuf>>,
    locked: AtomicBool,
}

impl ObservedRemoval {
    fn new(w: &World) -> Arc<Self> {
        Arc::new(Self { inner: w.installed.clone(), web: w.web.clone(), served_at_removal: Mutex::default(), locked: AtomicBool::new(false) })
    }

    fn served_at_removal(&self) -> Option<PathBuf> {
        self.served_at_removal.lock().unwrap().clone()
    }
}

#[async_trait]
impl InstalledPackageRegistry for ObservedRemoval {
    fn list(&self) -> PortResult<Vec<InstalledPackage>> {
        self.inner.list()
    }
    fn install_dir(&self, id: &PackageId) -> PathBuf {
        self.inner.install_dir(id)
    }
    fn record(&self, package: &InstalledPackage) -> PortResult<()> {
        self.inner.record(package)
    }
    async fn remove(&self, id: &PackageId) -> PortResult<()> {
        *self.served_at_removal.lock().unwrap() = served_php_cgi(&self.web);
        if self.locked.load(Ordering::SeqCst) {
            return Err(PortError::io("removing C:\\lampo\\packages\\php-8.3.15", "the folder is in use"));
        }
        self.inner.remove(id).await
    }
}

fn served_php_cgi(web: &FakeWeb) -> Option<PathBuf> {
    web.last().and_then(|config| config.vhosts.first().map(|v| v.php_cgi.clone()))
}

fn php(version: &str) -> PackageIdDto {
    PackageIdDto { kind: "php".into(), version: version.into(), php_version: None }
}

/// `shop` (PHP 8.3) served from 8.3.15, with 8.3.14 still installed.
async fn shop_served_by_the_newest_patch() -> World {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.installed.add(PackageKind::PHP, "8.3.15", None);
    w.hosts_in_sync();
    w.env.sync(&gui()).await.unwrap();
    assert_eq!(served_php_cgi(&w.web).as_deref(), Some(Path::new(PHP_8_3_15_CGI)));
    w.clear_calls();
    w
}

#[tokio::test]
async fn uninstalling_the_patch_a_site_uses_moves_apache_to_the_remaining_build_first() {
    let w = shop_served_by_the_newest_patch().await;
    let registry = ObservedRemoval::new(&w);

    w.uninstall_package_with(registry.clone()).execute(php("8.3.15"), &gui()).await.unwrap();

    assert_eq!(registry.served_at_removal().as_deref(), Some(Path::new(PHP_8_3_14_CGI)), "the vhost left 8.3.15 before its files were deleted");
    assert_eq!(w.apache.calls(), ["prepare:apache", "restart:apache"], "the FastCGI processes of 8.3.15 are gone too");
    assert!(!w.installed.has(&PackageKind::PHP, "8.3.15"));
    assert!(w.installed.has(&PackageKind::PHP, "8.3.14"));
}

#[tokio::test]
async fn a_removal_that_fails_puts_the_environment_back_on_the_package() {
    let w = shop_served_by_the_newest_patch().await;
    let registry = ObservedRemoval::new(&w);
    registry.locked.store(true, Ordering::SeqCst);

    let err = w.uninstall_package_with(registry.clone()).execute(php("8.3.15"), &gui()).await.unwrap_err();

    assert_eq!(err.code, ErrorCode::Io);
    assert_eq!(registry.served_at_removal().as_deref(), Some(Path::new(PHP_8_3_14_CGI)));
    assert_eq!(served_php_cgi(&w.web).as_deref(), Some(Path::new(PHP_8_3_15_CGI)), "still installed, so still the newest build of 8.3");
    assert!(w.installed.has(&PackageKind::PHP, "8.3.15"));
}

#[tokio::test]
async fn a_refused_uninstall_touches_nothing() {
    let w = shop_served_by_the_newest_patch().await;
    w.uninstall_package().execute(php("8.3.15"), &gui()).await.unwrap();
    w.clear_calls();
    let applied = w.web.applied.lock().unwrap().len();

    let err = w.uninstall_package().execute(php("8.3.14"), &gui()).await.unwrap_err();

    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(w.web.applied.lock().unwrap().len(), applied, "no sync for a package that stays");
    assert!(w.all_calls().is_empty());
}
