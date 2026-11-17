//! Uninstall moves what uses a package to the build that remains before its files go.
//!
//! One build per series (one slot each): the build that remains is of another series. Redis
//! fits: a site that uses it without naming a series runs the newest one installed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lampo_domain::package::{InstalledPackage, PackageId, PackageKind};
use lampo_domain::ports::InstalledPackageRegistry;
use lampo_domain::service::{ServiceId, ServiceKind};
use lampo_domain::version::VersionSeries;
use lampo_domain::{PortError, PortResult};

use super::gui;
use crate::context::CallContext;
use crate::dto::{PackageIdDto, SyncReportDto};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::testing::{FakeRunner, MemInstalled, World};
use crate::use_cases::EnvironmentSync;

const REDIS_8_10: &str = "C:\\lampo\\packages\\redis-8.10";
const REDIS_5_0: &str = "C:\\lampo\\packages\\redis-5.0";

/// The world's registry, recording which build Redis ran from when the files were deleted (the
/// moment a Windows lock would bite). Optionally refuses the removal, like a locked folder.
struct ObservedRemoval {
    inner: Arc<MemInstalled>,
    redis: Arc<FakeRunner>,
    running_at_removal: Mutex<Option<PathBuf>>,
    locked: AtomicBool,
}

impl ObservedRemoval {
    fn new(w: &World) -> Arc<Self> {
        Arc::new(Self { inner: w.installed.clone(), redis: w.redis.clone(), running_at_removal: Mutex::default(), locked: AtomicBool::new(false) })
    }

    fn running_at_removal(&self) -> Option<PathBuf> {
        self.running_at_removal.lock().unwrap().clone()
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
        *self.running_at_removal.lock().unwrap() = redis_build(&self.redis);
        if self.locked.load(Ordering::SeqCst) {
            return Err(PortError::io(format!("removing {REDIS_8_10} (is a service using it? stop it and retry)"), "the folder is in use"));
        }
        self.inner.remove(id).await
    }
}

/// The build the last Redis launch ran from.
fn redis_build(redis: &FakeRunner) -> Option<PathBuf> {
    redis.launches.lock().unwrap().last().map(|launch| launch.install_dir.clone())
}

fn redis(version: &str) -> PackageIdDto {
    PackageIdDto { kind: "redis".into(), version: version.into(), php_version: None }
}

/// `shop` uses Redis without naming a series, so it runs from 8.10, the newest; 5.0 stays installed.
async fn shop_on_the_newest_redis() -> World {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.update_project("shop", |p| p.environment.use_service(ServiceKind::REDIS, None));
    w.installed.add(PackageKind::REDIS, "8.10.3", None);
    w.hosts_in_sync();
    w.env.sync(&gui()).await.unwrap();
    assert_eq!(redis_build(&w.redis).as_deref(), Some(Path::new(REDIS_8_10)));
    w.clear_calls();
    w
}

#[tokio::test]
async fn uninstalling_the_build_a_service_runs_from_moves_it_to_the_remaining_build_first() {
    let w = shop_on_the_newest_redis().await;
    let registry = ObservedRemoval::new(&w);

    w.uninstall_package_with(registry.clone(), w.env.clone()).execute(redis("8.10.3"), &gui()).await.unwrap();

    assert_eq!(registry.running_at_removal().as_deref(), Some(Path::new(REDIS_5_0)), "Redis left 8.10 before its files were deleted");
    assert_eq!(w.redis.calls(), ["prepare:redis", "restart:redis"]);
    assert!(!w.installed.has(&PackageKind::REDIS, "8.10.3"));
    assert!(w.installed.has(&PackageKind::REDIS, "5.0.14.1"));
}

#[tokio::test]
async fn a_removal_that_fails_puts_the_environment_back_on_the_package() {
    let w = shop_on_the_newest_redis().await;
    let registry = ObservedRemoval::new(&w);
    registry.locked.store(true, Ordering::SeqCst);

    let err = w.uninstall_package_with(registry.clone(), w.env.clone()).execute(redis("8.10.3"), &gui()).await.unwrap_err();

    assert_eq!(err.code, ErrorCode::Io);
    assert!(err.message.contains("is a service using it?"), "nothing else explains the lock: {}", err.message);
    assert_eq!(registry.running_at_removal().as_deref(), Some(Path::new(REDIS_5_0)));
    assert_eq!(redis_build(&w.redis).as_deref(), Some(Path::new(REDIS_8_10)), "still installed, so still the newest build");
    assert!(w.installed.has(&PackageKind::REDIS, "8.10.3"));
}

#[tokio::test]
async fn a_refused_uninstall_touches_nothing() {
    let w = shop_on_the_newest_redis().await;
    w.uninstall_package().execute(redis("8.10.3"), &gui()).await.unwrap();
    w.clear_calls();
    let applied = w.web.applied.lock().unwrap().len();

    let err = w.uninstall_package().execute(redis("5.0.14.1"), &gui()).await.unwrap_err();

    assert_eq!((err.code, err.details.unwrap()["usedBy"].clone()), (ErrorCode::Conflict, serde_json::json!(["shop"])));
    assert_eq!(w.web.applied.lock().unwrap().len(), applied, "no sync for a package that stays");
    assert!(w.all_calls().is_empty());
}

/// The world's environment, with a hook that runs once the environment moved off the package.
struct AfterMoveOff {
    inner: Arc<dyn EnvironmentSync>,
    /// Returned instead of moving off, when set.
    fails_with: Option<AppError>,
    then: Box<dyn Fn() + Send + Sync>,
}

#[async_trait]
impl EnvironmentSync for AfterMoveOff {
    async fn sync(&self, ctx: &CallContext) -> AppResult<SyncReportDto> {
        self.inner.sync(ctx).await
    }
    async fn sync_without(&self, retiring: &PackageId, ctx: &CallContext) -> AppResult<SyncReportDto> {
        let moved = match &self.fails_with {
            Some(e) => Err(e.clone()),
            None => self.inner.sync_without(retiring, ctx).await,
        };
        (self.then)();
        moved
    }
}

/// The few seconds the services take to move are enough for a site to start needing the
/// package: it is checked again before its files go, and stays.
#[tokio::test]
async fn a_site_that_starts_needing_the_package_while_services_move_keeps_it() {
    let w = Arc::new(shop_on_the_newest_redis().await);
    let pinned = w.clone();
    let env = Arc::new(AfterMoveOff {
        inner: w.env.clone(),
        fails_with: None,
        then: Box::new(move || pinned.update_project("shop", |p| p.environment.use_service(ServiceKind::REDIS, Some(VersionSeries::parse("8.10").unwrap())))),
    });
    let registry = ObservedRemoval::new(&w);

    let err = w.uninstall_package_with(registry.clone(), env).execute(redis("8.10.3"), &gui()).await.unwrap_err();

    assert_eq!((err.code, err.details.unwrap()["usedBy"].clone()), (ErrorCode::Conflict, serde_json::json!(["shop"])));
    assert_eq!(registry.running_at_removal(), None, "the files were never touched");
    assert!(w.installed.has(&PackageKind::REDIS, "8.10.3"));
    let redis_8_10 = ServiceId::new(ServiceKind::REDIS, Some(VersionSeries::parse("8.10").unwrap()));
    assert!(w.redis.launches.lock().unwrap().last().is_some_and(|l| l.service == redis_8_10 && l.install_dir == Path::new(REDIS_8_10)), "synced back onto the package");
}

/// When the services could not be moved off the package, that failure is why its files are still
/// in use: it is the error reported, not the removal's guess.
#[tokio::test]
async fn a_failed_move_off_is_the_error_reported() {
    let w = shop_on_the_newest_redis().await;
    let env = Arc::new(AfterMoveOff { inner: w.env.clone(), fails_with: Some(AppError::new(ErrorCode::ProcessFailed, "redis-server.exe did not stop")), then: Box::new(|| {}) });
    let registry = ObservedRemoval::new(&w);
    registry.locked.store(true, Ordering::SeqCst);

    let err = w.uninstall_package_with(registry.clone(), env).execute(redis("8.10.3"), &gui()).await.unwrap_err();

    assert_eq!(err.code, ErrorCode::ProcessFailed);
    assert!(err.message.contains("redis-server.exe did not stop") && !err.message.contains("is a service using it?"), "{}", err.message);
    assert!(w.installed.has(&PackageKind::REDIS, "8.10.3"));
}
