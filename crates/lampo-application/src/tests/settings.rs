//! Settings saves: `PatchSettings` (`lampo settings set`, partial, atomic edits) and the GUI
//! page's full save (`SettingsService::update`), with the same lock and checks.

use std::collections::BTreeMap;

use lampo_domain::address::{Addressing, SiteAddress};
use lampo_domain::events::DomainEvent;
use lampo_domain::mysql::MysqlSeries;
use lampo_domain::ports::{ProjectRepository, SettingsRepository};
use lampo_domain::service::{ServiceId, ServiceState};
use lampo_domain::tcp_port::Port;

use super::allocation::after_a_locked_commit;
use super::{cli, gui};
use crate::dto::*;
use crate::error::ErrorCode;
use crate::testing::*;
use crate::use_cases::SiteSnapshot;

fn ports(pairs: &[(&str, u16)]) -> BTreeMap<String, u16> {
    pairs.iter().map(|(id, port)| (id.to_string(), *port)).collect()
}

/// Shop (active, MySQL 8.4) served; Apache and MySQL 8.4 running.
async fn served() -> World {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.hosts_in_sync();
    crate::use_cases::EnvironmentSync::sync(w.env.as_ref(), &cli()).await.unwrap();
    assert_eq!(w.apache.state(&ServiceId::apache()), ServiceState::Running);
    w.clear_calls();
    w
}

#[tokio::test]
async fn ports_change_in_place_and_running_services_restart() {
    let w = served().await;
    // A concurrent GUI save of another field is kept: the patch applies to the stored settings.
    w.settings.0.lock().unwrap().close_to_tray = false;
    let request = SettingsPatchRequest { apache_port: Some(8080), service_ports: ports(&[("mysql-8.4", 3310), ("redis", 6380)]), ..Default::default() };
    let result = w.patch_settings().execute(request, &cli()).await.unwrap();

    let stored = w.settings.load().unwrap();
    assert_eq!((stored.apache_port, stored.mysql_port(MysqlSeries::DEFAULT), stored.service_port(&redis_id())), (Port::of(8080), Some(Port::of(3310)), Some(Port::of(6380))));
    assert!(!stored.close_to_tray, "untouched fields keep their stored value");
    assert_eq!(result.settings.apache_port, 8080);
    let changes: Vec<(&str, &str, &str)> = result.changes.iter().map(|c| (c.setting.as_str(), c.from.as_str(), c.to.as_str())).collect();
    assert_eq!(changes, [("apache.port", "80", "8080"), ("mysql-8.4.port", "3306", "3310"), ("redis.port", "6379", "6380")]);
    assert!(w.events.all().contains(&DomainEvent::SettingsChanged));

    assert_eq!(result.sync.restarted, ["mysql-8.4", "apache"], "running services move to their new port");
    let mysql_launch = w.mysql.launches.lock().unwrap().last().cloned().unwrap();
    assert_eq!(mysql_launch.port, 3310);
    let notes = result.notes.join("\n");
    assert!(notes.contains("Redis is not running: it listens on port 6380 when it next starts"), "{notes}");
    assert!(notes.contains("Sites using MySQL 8.4: shop. Update DB_PORT to 3310"), "{notes}");
    assert!(notes.contains("Host-name URLs now use port 8080 (projects shop)"), "{notes}");
}

#[tokio::test]
async fn default_php_and_loopback_range() {
    let w = World::with_basics();
    let request = SettingsPatchRequest { default_php_version: Some("8.3".into()), loopback_port_range: Some(PortRangeDto { start: 9000, end: 9099 }), ..Default::default() };
    let result = w.patch_settings().execute(request, &cli()).await.unwrap();
    assert_eq!(result.settings.default_php_version.as_deref(), Some("8.3"));
    assert_eq!(result.settings.loopback_port_range, PortRangeDto { start: 9000, end: 9099 });
    let settings: Vec<&str> = result.changes.iter().map(|c| c.setting.as_str()).collect();
    assert_eq!(settings, ["default-php", "loopback-port-range"]);
    assert!(result.notes.is_empty());

    let missing = w.patch_settings().execute(SettingsPatchRequest { default_php_version: Some("8.2".into()), ..Default::default() }, &cli()).await.unwrap_err();
    assert_eq!(missing.code, ErrorCode::PackageMissing);
    assert!(missing.message.contains("lampo package install php 8.2"), "{}", missing.message);
    let bad_range = w.patch_settings().execute(SettingsPatchRequest { loopback_port_range: Some(PortRangeDto { start: 9100, end: 9000 }), ..Default::default() }, &cli()).await.unwrap_err();
    assert_eq!(bad_range.code, ErrorCode::Validation);
}

#[tokio::test]
async fn the_default_addressing_is_saved_without_a_sync() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.hosts_in_sync();
    assert_eq!(w.settings_service().get().unwrap().default_addressing, AddressingDto::NoElevation, "the built-in default");

    let request = SettingsPatchRequest { default_addressing: Some(AddressingDto::HostsOnly), ..Default::default() };
    let result = w.patch_settings().execute(request, &cli()).await.unwrap();
    assert_eq!(result.settings.default_addressing, AddressingDto::HostsOnly);
    let changes: Vec<(&str, &str, &str)> = result.changes.iter().map(|c| (c.setting.as_str(), c.from.as_str(), c.to.as_str())).collect();
    assert_eq!(changes, [("default-addressing", "no-elevation", "hosts-only")]);
    assert_eq!(w.settings.load().unwrap().default_addressing, Addressing::HostsOnly);
    assert!(w.events.all().contains(&DomainEvent::SettingsChanged));
    assert_eq!(result.sync, SyncReportDto::default(), "only new projects are affected");
    assert!(w.all_calls().is_empty(), "the pending sync of `shop` was not run: {:?}", w.all_calls());

    let again = w.patch_settings().execute(SettingsPatchRequest { default_addressing: Some(AddressingDto::HostsOnly), ..Default::default() }, &cli()).await.unwrap();
    assert!(again.changes.is_empty());
}

#[tokio::test]
async fn the_automatic_restart_is_switched_off_without_a_sync() {
    let w = served().await;
    assert!(w.settings_service().get().unwrap().auto_restart_services, "on by default");

    let request = SettingsPatchRequest { auto_restart_services: Some(false), ..Default::default() };
    let result = w.patch_settings().execute(request, &cli()).await.unwrap();
    assert!(!result.settings.auto_restart_services);
    let changes: Vec<(&str, &str, &str)> = result.changes.iter().map(|c| (c.setting.as_str(), c.from.as_str(), c.to.as_str())).collect();
    assert_eq!(changes, [("auto-restart", "on", "off")]);
    assert!(!w.settings.load().unwrap().auto_restart_services);
    assert_eq!(result.sync, SyncReportDto::default(), "read when a service crashes");
    assert!(w.all_calls().is_empty(), "{:?}", w.all_calls());
}

#[tokio::test]
async fn unchanged_values_are_a_no_op_without_a_sync() {
    let w = served().await;
    let before = w.events.all().len();
    let result = w.patch_settings().execute(SettingsPatchRequest { apache_port: Some(80), service_ports: ports(&[("redis", 6379)]), ..Default::default() }, &cli()).await.unwrap();
    assert!(result.changes.is_empty() && result.notes.is_empty());
    assert_eq!(result.sync, SyncReportDto::default());
    assert!(w.all_calls().is_empty(), "nothing restarted: {:?}", w.all_calls());
    assert_eq!(w.events.all().len(), before);
}

#[tokio::test]
async fn invalid_patches_are_rejected_and_nothing_is_saved() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.update_project("shop", |p| add_address(p, SiteAddress::LoopbackPort(Port::of(8101))));
    let original = w.settings.load().unwrap();
    let patch = w.patch_settings();
    let cases: Vec<(SettingsPatchRequest, ErrorCode, &str)> = vec![
        (SettingsPatchRequest::default(), ErrorCode::Validation, "nothing to change"),
        (SettingsPatchRequest { service_ports: ports(&[("redis", 3306)]), ..Default::default() }, ErrorCode::Conflict, "port 3306 is assigned to both"),
        (SettingsPatchRequest { apache_port: Some(3307), ..Default::default() }, ErrorCode::Conflict, "3307"),
        (SettingsPatchRequest { service_ports: ports(&[("mysql-8.0", 3310)]), ..Default::default() }, ErrorCode::Validation, "configured: mysql-8.4, mysql-5.7, redis"),
        (SettingsPatchRequest { service_ports: ports(&[("apache", 8080)]), ..Default::default() }, ErrorCode::Validation, "--apache-port"),
        (SettingsPatchRequest { service_ports: ports(&[("my_sql", 1)]), ..Default::default() }, ErrorCode::Validation, "invalid service id"),
        (SettingsPatchRequest { apache_port: Some(0), ..Default::default() }, ErrorCode::Validation, "invalid port 0"),
        (SettingsPatchRequest { service_ports: ports(&[("redis", 8101)]), ..Default::default() }, ErrorCode::Conflict, "site `shop`"),
    ];
    for (request, code, needle) in cases {
        let e = patch.execute(request.clone(), &cli()).await.unwrap_err();
        assert_eq!(e.code, code, "{request:?}: {e:?}");
        assert!(e.message.contains(needle), "{request:?}: {}", e.message);
    }
    assert_eq!(w.settings.load().unwrap(), original);
    assert!(!w.events.all().contains(&DomainEvent::SettingsChanged));
}

#[tokio::test]
async fn a_new_port_must_be_free_unless_a_lampo_service_holds_it() {
    let w = served().await;
    w.ports.busy(8080, "skype.exe (pid 42)");
    let e = w.patch_settings().execute(SettingsPatchRequest { apache_port: Some(8080), ..Default::default() }, &cli()).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::PortInUse);
    assert_eq!(e.details.unwrap()["owner"], "skype.exe (pid 42)");
    assert_eq!(w.settings.load().unwrap().apache_port, 80);

    // Swapping two instances' ports: both are Lampo's own, so the probe is not consulted.
    w.ports.busy(3306, "mysqld.exe");
    w.ports.busy(3307, "mysqld.exe");
    let swap = SettingsPatchRequest { service_ports: ports(&[("mysql-8.4", 3307), ("mysql-5.7", 3306)]), ..Default::default() };
    let result = w.patch_settings().execute(swap, &cli()).await.unwrap();
    assert_eq!(result.changes.len(), 2);
    let stored = w.settings.load().unwrap();
    assert_eq!((stored.mysql_port(MysqlSeries::DEFAULT), stored.mysql_port(MysqlSeries::LEGACY_5_7)), (Some(Port::of(3307)), Some(Port::of(3306))));
}

// --- GUI save (`SettingsService::update`): same lock and checks as `lampo settings set` --------

/// Shop holds the dedicated loopback port 8101.
fn with_loopback_site() -> World {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.update_project("shop", |p| add_address(p, SiteAddress::LoopbackPort(Port::of(8101))));
    w.hosts_in_sync();
    w
}

fn instance<'a>(dto: &'a mut SettingsDto, id: &str) -> &'a mut ServiceInstanceDto {
    dto.services.iter_mut().find(|s| s.service == id).unwrap()
}

/// The save is refused with `code` (and a message containing `needle`); nothing is saved.
async fn assert_gui_save_refused(w: &World, dto: SettingsDto, code: ErrorCode, needle: &str) -> crate::error::AppError {
    let original = w.settings.load().unwrap();
    let e = w.settings_service().update(dto, &gui()).await.unwrap_err();
    assert_eq!(e.code, code, "{e:?}");
    assert!(e.message.contains(needle), "{}", e.message);
    assert_eq!(w.settings.load().unwrap(), original, "nothing saved");
    assert!(!w.events.all().contains(&DomainEvent::SettingsChanged));
    e
}

#[tokio::test]
async fn gui_save_refuses_a_sites_loopback_port() {
    let w = with_loopback_site();
    let mut dto = w.settings_service().get().unwrap();
    instance(&mut dto, "redis").port = 8101;
    assert_gui_save_refused(&w, dto, ErrorCode::Conflict, "site `shop`").await;
}

#[tokio::test]
async fn gui_save_refuses_a_port_another_program_listens_on() {
    let w = World::with_basics();
    w.ports.busy(8080, "skype.exe (pid 42)");
    let mut dto = w.settings_service().get().unwrap();
    dto.apache_port = 8080;
    let e = assert_gui_save_refused(&w, dto, ErrorCode::PortInUse, "8080").await;
    assert_eq!(e.details.unwrap()["owner"], "skype.exe (pid 42)");
}

#[tokio::test]
async fn gui_save_refuses_a_default_php_that_is_not_installed() {
    let w = World::with_basics();
    let mut dto = w.settings_service().get().unwrap();
    dto.default_php_version = Some("8.2".into());
    assert_gui_save_refused(&w, dto, ErrorCode::PackageMissing, "lampo package install php 8.2").await;

    // An unchanged default PHP is not re-checked (it may have been uninstalled meanwhile).
    w.settings.0.lock().unwrap().default_php = Some(php("8.2"));
    let mut dto = w.settings_service().get().unwrap();
    dto.close_to_tray = !dto.close_to_tray;
    let saved = w.settings_service().update(dto, &gui()).await.unwrap();
    assert_eq!(saved.default_php_version.as_deref(), Some("8.2"));
}

#[tokio::test]
async fn gui_save_sets_the_default_addressing() {
    let w = World::with_basics();
    let mut dto = w.settings_service().get().unwrap();
    dto.default_addressing = AddressingDto::Standard;
    let saved = w.settings_service().update(dto, &gui()).await.unwrap();
    assert_eq!(saved.default_addressing, AddressingDto::Standard);
    assert_eq!(w.settings.load().unwrap().default_addressing, Addressing::Standard);
}

#[tokio::test]
async fn gui_save_sets_the_automatic_restart() {
    let w = World::with_basics();
    let mut dto = w.settings_service().get().unwrap();
    dto.auto_restart_services = false;
    assert!(!w.settings_service().update(dto, &gui()).await.unwrap().auto_restart_services);
    assert!(!w.settings.load().unwrap().auto_restart_services);
}

#[tokio::test]
async fn gui_save_cannot_add_or_remove_service_instances() {
    let w = World::with_basics();
    let current = w.settings_service().get().unwrap();

    let mut outdated = current.clone();
    outdated.services.retain(|s| s.service != "mysql-5.7");
    assert_gui_save_refused(&w, outdated, ErrorCode::Validation, "mysql-5.7 is missing").await;

    let mut added = current.clone();
    let mut extra = added.services[0].clone();
    (extra.service, extra.port) = ("mysql-8.0".into(), 3310);
    added.services.push(extra);
    assert_gui_save_refused(&w, added, ErrorCode::Validation, "mysql-8.0 is not a configured service instance").await;

    let mut twice = current.clone();
    let mut duplicate = twice.services[2].clone();
    duplicate.port = 6380;
    twice.services.push(duplicate);
    assert_gui_save_refused(&w, twice, ErrorCode::Validation, "redis is given twice").await;

    // The instances may come in any order: the stored order is kept.
    let mut reordered = current.clone();
    reordered.services.reverse();
    instance(&mut reordered, "redis").port = 6380;
    let saved = w.settings_service().update(reordered, &gui()).await.unwrap();
    let ids: Vec<&str> = saved.services.iter().map(|s| s.service.as_str()).collect();
    assert_eq!(ids, current.services.iter().map(|s| s.service.as_str()).collect::<Vec<_>>());
    assert_eq!(w.settings.load().unwrap().service_port(&redis_id()), Some(Port::of(6380)));
}

/// The GUI save takes the allocator lock, so a project saved meanwhile under the lock is part of
/// its checks: the port the GUI wanted is now that site's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gui_save_waits_for_the_allocator_lock() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.hosts_in_sync();
    let service = w.settings_service();
    let mut dto = service.get().unwrap();
    instance(&mut dto, "redis").port = 8101;
    let projects = w.projects.clone();
    let take_8101 = move |snap: &SiteSnapshot| {
        let mut shop = snap.projects[0].clone();
        add_address(&mut shop, SiteAddress::LoopbackPort(Port::of(8101)));
        Ok(projects.save(&shop)?)
    };
    let saved = after_a_locked_commit(&w, take_8101, async move { service.update(dto, &gui()).await }).await;
    assert_eq!(saved.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(w.settings.load().unwrap().service_port(&redis_id()), Some(Port::of(6379)));
}
