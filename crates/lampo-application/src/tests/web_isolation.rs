//! Isolating the vhosts Apache rejects by bisection, and reporting every sync failure.

use std::sync::atomic::Ordering;

use lampo_domain::project::ApacheDirectives;
use lampo_domain::service::{ServiceId, ServiceState};

use super::{cli, gui};
use crate::error::ErrorCode;
use crate::testing::World;
use crate::use_cases::EnvironmentSync;

/// Validations of one isolation among this many sites: the full set, the base without vhosts,
/// two halves per bisection level (5 for 32 sites) and the final apply. One per site before.
const SITES: usize = 32;
const MAX_VALIDATIONS_FOR_ONE_BROKEN_SITE: usize = 13;

fn sites(w: &World, broken: &[usize]) -> Vec<String> {
    let ids: Vec<String> = (0..SITES).map(|i| format!("site{i:02}")).collect();
    for (i, id) in ids.iter().enumerate() {
        w.project(id, &format!("C:\\sites\\{id}"), true);
        if broken.contains(&i) {
            w.update_project(id, |p| p.environment.apache_directives = ApacheDirectives::new("Broken directive").unwrap());
        }
    }
    w.hosts_in_sync();
    ids
}

fn served(w: &World) -> Vec<String> {
    w.web.last().unwrap().vhosts.iter().map(|v| v.site.to_string()).collect()
}

#[tokio::test]
async fn one_broken_site_among_many_is_found_with_a_few_validations() {
    let w = World::with_basics();
    let ids = sites(&w, &[19]);

    let err = w.env.sync(&gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigRejected);
    let details = err.details.unwrap();
    assert_eq!(details["excludedSites"], serde_json::json!(["site19"]));
    assert!(details["output"].as_str().unwrap().contains("site19"));
    let healthy: Vec<String> = ids.iter().filter(|id| *id != "site19").cloned().collect();
    assert_eq!(served(&w), healthy);
    assert!(w.web.attempts.load(Ordering::SeqCst) <= MAX_VALIDATIONS_FOR_ONE_BROKEN_SITE, "{} validations", w.web.attempts.load(Ordering::SeqCst));
    assert_eq!(w.apache.state(&ServiceId::apache()), ServiceState::Running);
}

#[tokio::test]
async fn several_broken_sites_are_all_left_out_in_order() {
    let w = World::with_basics();
    let ids = sites(&w, &[0, 7, 8, 31]);

    let details = w.env.sync(&gui()).await.unwrap_err().details.unwrap();
    assert_eq!(details["excludedSites"], serde_json::json!(["site00", "site07", "site08", "site31"]));
    let rejections = details["rejections"].as_array().unwrap();
    assert!(rejections.iter().all(|r| r["output"].as_str().unwrap().contains(r["site"].as_str().unwrap())), "each site keeps its own output: {rejections:?}");
    assert_eq!(served(&w).len(), ids.len() - 4);
}

#[tokio::test]
async fn a_configuration_rejected_without_any_site_is_not_bisected() {
    let w = World::with_basics();
    sites(&w, &[]);
    w.web.reject_all.store(true, Ordering::SeqCst);

    assert_eq!(w.env.sync(&gui()).await.unwrap_err().code, ErrorCode::ConfigRejected);
    assert_eq!(w.web.attempts.load(Ordering::SeqCst), 2, "the full configuration, then the one without vhosts");
    assert_eq!(w.apache.calls(), Vec::<String>::new());
}

#[tokio::test]
async fn every_failure_of_a_sync_is_listed_with_its_code() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.update_project("shop", |p| p.environment.apache_directives = ApacheDirectives::new("Broken directive").unwrap());

    // Apache rejects `shop`, then the hosts file needs elevation: both are reported.
    let err = w.env.sync(&cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigRejected, "the first failure is the error");
    let codes: Vec<String> = err.details.unwrap()["errors"].as_array().unwrap().iter().map(|e| e["code"].as_str().unwrap().to_string()).collect();
    assert_eq!(codes, ["config-rejected", "elevation-required"]);
}
