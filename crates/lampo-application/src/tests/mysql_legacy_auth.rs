//! PHP before 7.4 on MySQL 8.4: the project's account (or root, for a project on root) moves to
//! `mysql_native_password` on create, PHP switch and service change; the answer and the doctor
//! warn, with MySQL 5.7 as the alternative.

use std::sync::Arc;

use lampo_domain::mysql::accounts::MysqlCredentials;
use lampo_domain::package::PackageKind;

use super::cli;
use crate::dto::*;
use crate::testing::*;
use crate::use_cases::*;

fn world() -> World {
    let w = World::with_basics();
    w.installed.add(PackageKind::PHP, "7.2.34", None);
    w.hosts_in_sync();
    w
}

fn dedicated(w: &World, id: &str) {
    w.project(id, &format!(r"C:\sites\{id}"), true);
    w.update_project(id, |p| p.mysql_credentials = MysqlCredentials::Project);
    w.hosts_in_sync();
}

fn switcher(w: &World) -> SetProjectPhp {
    SetProjectPhp::new(w.project_writer(), w.installed.clone())
}

fn codes(dto: &ProjectDto) -> Vec<ProjectWarningCodeDto> {
    dto.warnings.iter().map(|warning| warning.code).collect()
}

fn last_accounts_call(w: &World) -> String {
    w.mysql_accounts.calls().into_iter().rev().find(|c| c.starts_with("accounts:")).unwrap()
}

#[tokio::test]
async fn creating_a_php_7_2_project_on_mysql_8_4_warns_and_gives_it_a_native_password_account() {
    let w = world();
    w.fs.add(r"C:\sites\legacy");
    let create = CreateProject::new(w.project_writer(), w.presets(), Arc::new(DetectProject::new(Vec::new(), w.projects.clone(), w.fs.clone())), w.fs.clone(), w.database_service());
    let request = CreateProjectRequest {
        root: r"C:\sites\legacy".into(),
        preset_id: Some("laravel".into()),
        php_version: Some("7.2".into()),
        addressing: AddressingDto::NoElevation,
        ..Default::default()
    };
    let created = create.execute(request, &cli()).await.unwrap();

    assert_eq!(codes(&created.project), [ProjectWarningCodeDto::LegacyPhpMysqlAuth]);
    let message = &created.project.warnings[0].message;
    assert!(message.contains("PHP 7.2") && message.contains("MySQL 8.4") && message.contains("mysql@5.7"), "{message}");
    assert_eq!(last_accounts_call(&w), "accounts:3306:lampo_legacy@native=legacy");
}

#[tokio::test]
async fn switching_php_moves_the_account_to_native_password_and_back() {
    let w = world();
    dedicated(&w, "shop");
    let switcher = switcher(&w);
    let request = |php: &str| SetProjectPhpRequest { project_id: "shop".into(), php_version: php.into() };

    let dto = switcher.execute(request("7.2"), &cli()).await.unwrap();
    assert_eq!(codes(&dto), [ProjectWarningCodeDto::LegacyPhpMysqlAuth]);
    assert_eq!(last_accounts_call(&w), "accounts:3306:lampo_shop@native=shop");

    let dto = switcher.execute(request("8.3"), &cli()).await.unwrap();
    assert!(dto.warnings.is_empty(), "{:?}", dto.warnings);
    assert_eq!(last_accounts_call(&w), "accounts:3306:lampo_shop=shop", "back to MySQL 8's default");
}

#[tokio::test]
async fn moving_the_project_to_mysql_5_7_clears_the_warning() {
    let w = world();
    dedicated(&w, "shop");
    let dto = w.patch_project().execute(ProjectPatchRequest { project_id: "shop".into(), php_version: Some("7.2".into()), ..Default::default() }, &cli()).await.unwrap();
    assert_eq!(codes(&dto), [ProjectWarningCodeDto::LegacyPhpMysqlAuth], "`project set --php` warns too");

    let to_5_7 = ProjectPatchRequest { project_id: "shop".into(), services_add: vec![ServiceUseDto { kind: "mysql".into(), version: Some("5.7".into()) }], ..Default::default() };
    let dto = w.patch_project().execute(to_5_7, &cli()).await.unwrap();
    assert!(dto.warnings.is_empty(), "{:?}", dto.warnings);
    assert!(w.mysql_accounts.calls().contains(&"accounts:3307:lampo_shop=shop".to_string()), "5.7 keeps its default: {:?}", w.mysql_accounts.calls());
}

#[tokio::test]
async fn a_php_7_2_project_on_root_moves_root_to_native_password() {
    let w = world();
    w.project("legacy", r"C:\sites\legacy", true);
    w.hosts_in_sync();
    let dto = switcher(&w).execute(SetProjectPhpRequest { project_id: "legacy".into(), php_version: "7.2".into() }, &cli()).await.unwrap();

    assert_eq!(codes(&dto), [ProjectWarningCodeDto::LegacyPhpMysqlAuth]);
    let calls = w.mysql_accounts.calls();
    assert!(calls.contains(&"native-root:3306".to_string()), "{calls:?}");
    assert!(!calls.iter().any(|c| c.starts_with("secure-root")), "root keeps no password while a project uses it: {calls:?}");
}

#[tokio::test]
async fn a_modern_project_on_root_leaves_root_alone() {
    let w = world();
    w.project("shop", r"C:\sites\shop", true);
    w.hosts_in_sync();
    w.env.sync(&cli()).await.unwrap();
    assert!(!w.mysql_accounts.calls().iter().any(|c| c.starts_with("native-root")), "{:?}", w.mysql_accounts.calls());
    assert!(w.project_queries().get("shop").unwrap().warnings.is_empty());
}

#[tokio::test]
async fn the_doctor_warns_about_old_php_on_mysql_8_only() {
    let w = world();
    w.project("shop", r"C:\sites\shop", true);
    w.project("legacy", r"C:\sites\legacy", false);
    w.update_project("legacy", |p| p.environment.php = php("7.2"));
    let doctor = Doctor::new(w.doctor_deps(Arc::new(FakeLocator::default())));
    let line = |report: &DoctorReportDto| report.checks.iter().find(|c| c.id == "mysql-auth").cloned().unwrap();

    let report = doctor.run().await.unwrap();
    let check = line(&report);
    assert_eq!(check.status, CheckStatus::Warning);
    assert!(check.message.ends_with("legacy (PHP 7.2, MySQL 8.4)"), "{}", check.message);
    assert!(check.fix.as_deref().is_some_and(|fix| fix.contains("--service mysql@5.7")), "{:?}", check.fix);

    w.update_project("legacy", |p| p.environment.set_mysql(Some(lampo_domain::mysql::MysqlSeries::LEGACY_5_7)));
    assert_eq!(line(&doctor.run().await.unwrap()).status, CheckStatus::Ok);
}
