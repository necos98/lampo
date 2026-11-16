//! `ShimToolQuery::which`: the shim's own resolution and invocation, with the reason spelled out.

use std::path::Path;
use std::sync::Arc;

use lampo_domain::package::PackageKind;
use lampo_domain::php::PhpMinor;
use lampo_domain::shim::{FolderProbe, FolderState, tools};

use crate::dto::*;
use crate::error::ErrorCode;
use crate::testing::World;
use crate::use_cases::ShimToolQuery;

/// Every folder exists (the site's database included).
struct EveryFolder;
impl FolderProbe for EveryFolder {
    fn state(&self, _path: &Path) -> FolderState {
        FolderState::Present
    }
}

/// `shop` (PHP 8.3, MySQL 8.4) and `blog` registered; default PHP 8.4.
fn world() -> World {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.project("blog", "C:\\sites\\blog", false);
    w.env.write_generated_files().unwrap();
    w
}

fn query(w: &World) -> ShimToolQuery {
    ShimToolQuery::new(w.resolver(), tools::all(), Arc::new(EveryFolder))
}

fn at(cwd: &str) -> ResolveRequestDto {
    ResolveRequestDto { cwd: cwd.into(), ..Default::default() }
}

fn inherited(cwd: &str, site: &str, root: &str) -> ResolveRequestDto {
    ResolveRequestDto { cwd: cwd.into(), inherited_site: Some(site.into()), inherited_root: Some(root.into()), ..Default::default() }
}

#[test]
fn php_in_a_project_folder_is_the_projects_binary() {
    let w = world();
    let which = query(&w).which(&at("C:\\sites\\shop\\public"), "php").unwrap();
    assert_eq!((which.tool.as_str(), which.reason, which.php_version.as_str()), ("php", ResolutionReasonDto::Directory, "8.3"));
    assert_eq!((which.resolution.site.as_deref(), which.resolution.root.as_deref()), (Some("shop"), Some("C:\\sites\\shop")));
    assert!(which.binary.ends_with("php.exe") && which.binary.contains("8.3"), "{}", which.binary);
    assert!(which.default_args.is_empty() && which.notices.is_empty(), "{which:?}");
}

#[test]
fn composer_runs_through_the_projects_php_and_mysql_selects_the_site_database() {
    let w = world();
    let q = query(&w);
    let composer = q.which(&at("C:\\sites\\shop"), "composer").unwrap();
    assert!(composer.binary.ends_with("php.exe"), "{}", composer.binary);
    assert!(composer.default_args[0].ends_with("composer.phar"), "{:?}", composer.default_args);

    let mysql = q.which(&at("C:\\sites\\shop"), "mysql").unwrap();
    assert!(mysql.binary.ends_with("mysql.exe"), "{}", mysql.binary);
    assert!(mysql.default_args.contains(&"--port=3306".to_string()), "{:?}", mysql.default_args);
    assert!(mysql.default_args.contains(&"--database=shop".to_string()), "{:?}", mysql.default_args);
}

#[test]
fn the_reason_tells_the_project_flag_from_lampo_project() {
    let w = world();
    let q = query(&w);
    let flag = ResolveRequestDto { cwd: "C:\\sites\\shop".into(), explicit_site: Some("blog".into()), ..Default::default() };
    let by_flag = q.which(&flag, "php").unwrap();
    assert_eq!((by_flag.resolution.site.as_deref(), by_flag.reason), (Some("blog"), ResolutionReasonDto::ProjectFlag));

    let by_variable = q.which(&inherited("E:\\scratch", "blog", "C:\\sites\\blog"), "php").unwrap();
    assert_eq!((by_variable.resolution.site.as_deref(), by_variable.reason), (Some("blog"), ResolutionReasonDto::LampoProject));
    assert!(by_variable.notices.is_empty());
}

#[test]
fn an_ignored_lampo_project_is_the_folder_with_a_notice() {
    let w = world();
    let which = query(&w).which(&inherited("C:\\sites\\shop", "blog", "C:\\sites\\blog"), "php").unwrap();
    assert_eq!((which.resolution.site.as_deref(), which.reason), (Some("shop"), ResolutionReasonDto::Directory));
    assert_eq!(which.notices, ["LAMPO_PROJECT=blog ignored: this folder belongs to shop (set LAMPO_QUIET=1 to hide this note)"]);
}

#[test]
fn outside_every_project_is_the_global_default() {
    let w = world();
    let which = query(&w).which(&at("E:\\elsewhere"), "PHP").unwrap();
    assert_eq!((which.tool.as_str(), which.reason, which.php_version.as_str()), ("php", ResolutionReasonDto::GlobalDefault, "8.4"));
    assert_eq!((which.resolution.site, which.resolution.kind), (None, ResolvedKind::GlobalDefault));
}

#[test]
fn failures_name_the_cause() {
    let w = world();
    let q = query(&w);
    let unknown = q.which(&at("C:\\sites\\shop"), "node").unwrap_err();
    assert_eq!(unknown.code, ErrorCode::Validation);
    assert!(unknown.message.contains("php, composer, mysql, mysqldump, redis-cli"), "{}", unknown.message);
    let flag = ResolveRequestDto { cwd: "C:\\sites\\shop".into(), explicit_site: Some("nope".into()), ..Default::default() };
    assert_eq!(q.which(&flag, "php").unwrap_err().code, ErrorCode::NotFound);

    let empty = World::new();
    empty.env.write_generated_files().unwrap();
    assert_eq!(query(&empty).which(&at("E:\\x"), "php").unwrap_err().code, ErrorCode::ProjectNotResolved);
}

#[test]
fn a_tool_that_is_not_installed_is_package_missing() {
    let w = World::new();
    w.installed.add(PackageKind::PHP, "8.4.1", None);
    w.settings.0.lock().unwrap().default_php = Some(PhpMinor::parse("8.4").unwrap());
    w.env.write_generated_files().unwrap();
    let q = query(&w);
    assert_eq!(q.which(&at("E:\\x"), "php").unwrap().reason, ResolutionReasonDto::GlobalDefault);
    let missing = q.which(&at("E:\\x"), "composer").unwrap_err();
    assert_eq!(missing.code, ErrorCode::PackageMissing);
    assert!(missing.message.contains("Composer"), "{}", missing.message);
}
