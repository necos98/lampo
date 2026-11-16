//! Extension names on every project save: the ones the builds renamed across PHP minors (`gd` ↔
//! `gd2`) are stored as the project's PHP names them, and a module compiled into the build
//! (`bcmath`) is accepted with a notice and gets no php.ini line.

use std::sync::atomic::Ordering;

use lampo_domain::package::PackageKind;
use lampo_domain::php::ExtensionName;
use lampo_domain::ports::ProjectRepository;
use lampo_domain::project::ProjectId;

use super::cli;
use crate::dto::*;
use crate::error::ErrorCode;
use crate::testing::*;
use crate::use_cases::{EnvironmentSync, SetProjectPhp};

fn patch(id: &str) -> ProjectPatchRequest {
    ProjectPatchRequest { project_id: id.into(), ..Default::default() }
}

fn add(names: &[&str]) -> ProjectPatchRequest {
    ProjectPatchRequest { extensions_add: names.iter().map(|n| n.to_string()).collect(), ..patch("shop") }
}

/// The extensions the last sync wrote in the php.ini of `site`.
fn ini_extensions(w: &World, site: &str) -> Vec<String> {
    let specs = w.ini.0.lock().unwrap();
    let spec = specs.iter().find(|s| s.site.to_string() == site).expect("php.ini written");
    spec.extensions.iter().map(|e| e.name.to_string()).collect()
}

fn stored_extensions(w: &World, id: &str) -> Vec<String> {
    let project = w.projects.get(&ProjectId::new(id).unwrap()).unwrap().unwrap();
    project.environment.extensions.iter().map(ToString::to_string).collect()
}

/// `shop` on PHP 7.4, with that build installed next to the basics.
fn world_with_php74_shop() -> World {
    let w = World::with_basics();
    w.installed.add(PackageKind::PHP, "7.4.33", None);
    w.project("shop", r"C:\sites\shop", true);
    w.update_project("shop", |p| p.environment.php = php("7.4"));
    w.hosts_in_sync();
    w
}

#[tokio::test]
async fn gd_is_enabled_under_the_name_of_the_projects_php() {
    let w = world_with_php74_shop();
    let dto = w.patch_project().execute(add(&["gd", "intl"]), &cli()).await.unwrap();
    assert_eq!(dto.environment.extensions, ["gd2", "intl"]);
    assert_eq!(stored_extensions(&w, "shop"), ["gd2", "intl"]);
    assert_eq!(ini_extensions(&w, "shop"), ["gd2", "intl"]);

    // Removing it by either name removes it.
    let dto = w.patch_project().execute(ProjectPatchRequest { extensions_remove: vec!["gd".into()], ..patch("shop") }, &cli()).await.unwrap();
    assert_eq!(dto.environment.extensions, ["intl"]);

    // The other way round on PHP 8.x.
    w.project("blog", r"C:\sites\blog", true);
    w.hosts_in_sync();
    let dto = w.patch_project().execute(ProjectPatchRequest { extensions_add: vec!["php_gd2.dll".into()], ..patch("blog") }, &cli()).await.unwrap();
    assert_eq!(dto.environment.extensions, ["gd"]);
}

#[tokio::test]
async fn a_php_switch_renames_the_extensions_both_ways() {
    let w = world_with_php74_shop();
    w.patch_project().execute(add(&["gd"]), &cli()).await.unwrap();
    let set_php = |to: &str| SetProjectPhpRequest { project_id: "shop".into(), php_version: to.into() };
    let switcher = SetProjectPhp::new(w.project_writer(), w.installed.clone());

    let dto = switcher.execute(set_php("8.3"), &cli()).await.unwrap();
    assert_eq!(dto.environment.extensions, ["gd"]);
    assert_eq!(ini_extensions(&w, "shop"), ["gd"]);
    let dto = switcher.execute(set_php("7.4"), &cli()).await.unwrap();
    assert_eq!(dto.environment.extensions, ["gd2"]);
    assert_eq!(stored_extensions(&w, "shop"), ["gd2"]);
}

#[tokio::test]
async fn a_project_saved_with_the_other_name_still_loads_gd() {
    let w = world_with_php74_shop();
    // Stored before the rule, under the PHP 8 name.
    w.update_project("shop", |p| {
        p.environment.extensions.insert(ExtensionName::new("gd").unwrap());
    });
    w.env.sync(&cli()).await.unwrap();
    assert_eq!(ini_extensions(&w, "shop"), ["gd2"]);
}

#[tokio::test]
async fn a_compiled_in_extension_is_accepted_with_a_notice_and_left_out_of_php_ini() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", true);
    w.hosts_in_sync();

    let dto = w.patch_project().execute(add(&["bcmath", "intl"]), &cli()).await.unwrap();
    assert_eq!(dto.environment.extensions, ["bcmath", "intl"], "kept: another build may ship it as a DLL");
    assert_eq!(dto.notices, ["bcmath is built into PHP 8.3, nothing to enable"]);
    assert_eq!(ini_extensions(&w, "shop"), ["intl"]);

    // The notice comes with the answer of a save only.
    assert!(w.project_queries().get("shop").unwrap().notices.is_empty());
}

#[tokio::test]
async fn an_unknown_extension_is_still_refused_with_the_alternatives() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", true);
    w.hosts_in_sync();
    let before = w.projects.list().unwrap();

    let err = w.patch_project().execute(add(&["bcmath", "intll"]), &cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.message.contains("no bundled extension `intll` (available: gd, intl, mbstring"), "{}", err.message);
    assert_eq!(w.projects.list().unwrap(), before, "nothing saved");
}

#[tokio::test]
async fn the_build_is_asked_for_its_modules_only_when_a_name_has_no_dll() {
    let w = World::with_basics();
    w.project("shop", r"C:\sites\shop", true);
    w.hosts_in_sync();

    let dto = w.patch_project().execute(add(&["gd", "intl"]), &cli()).await.unwrap();
    assert!(dto.notices.is_empty());
    assert_eq!(w.scanner.builtin_calls.load(Ordering::SeqCst), 0, "every name has a DLL: PHP is not run");

    w.patch_project().execute(add(&["json"]), &cli()).await.unwrap();
    assert!(w.scanner.builtin_calls.load(Ordering::SeqCst) > 0);
}
