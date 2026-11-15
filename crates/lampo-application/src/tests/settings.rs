//! `PatchSettings` (`lampo settings set`): partial, atomic edits of the global settings.

use std::collections::BTreeMap;

use lampo_domain::address::SiteAddress;
use lampo_domain::events::DomainEvent;
use lampo_domain::mysql::MysqlSeries;
use lampo_domain::ports::SettingsRepository;
use lampo_domain::service::{ServiceId, ServiceState};

use super::cli;
use crate::dto::*;
use crate::error::ErrorCode;
use crate::testing::*;

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
    assert_eq!((stored.apache_port, stored.mysql_port(MysqlSeries::DEFAULT), stored.service_port(&redis_id())), (8080, Some(3310), Some(6380)));
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
    w.update_project("shop", |p| p.addresses.push(SiteAddress::LoopbackPort(8101)));
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
    assert_eq!((stored.mysql_port(MysqlSeries::DEFAULT), stored.mysql_port(MysqlSeries::LEGACY_5_7)), (Some(3307), Some(3306)));
}
