//! A mutation reads the registry (projects, worktree environments, settings) once, under the
//! allocator lock, and hands that snapshot to the sync and the view after it; a snapshot that a
//! later commit overtook is read again instead of being applied.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use lampo_domain::progress::CancelFlag;

use super::gui;
use crate::dto::*;
use crate::testing::*;
use crate::use_cases::*;

/// The sites Apache serves, by id.
fn served(w: &World) -> Vec<String> {
    w.web.last().map(|config| config.vhosts.iter().map(|v| v.site.to_string()).collect()).unwrap_or_default()
}

fn activate(id: &str) -> SetProjectActiveRequest {
    SetProjectActiveRequest { project_id: id.into(), active: true }
}

#[tokio::test]
async fn a_mutation_reads_each_store_once() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.hosts_in_sync();
    w.reset_store_reads();

    let shop = SetProjectActive::new(w.project_writer()).execute(activate("shop"), &gui()).await.unwrap();

    assert!(shop.active);
    assert_eq!(served(&w), ["shop"], "the sync applied the saved project");
    assert_eq!(w.store_reads(), (1, 1, 1), "one snapshot under the lock; the sync and the view reuse it");
}

#[tokio::test]
async fn a_settings_save_reads_each_store_once() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.hosts_in_sync();
    w.reset_store_reads();

    let request = SettingsPatchRequest { apache_port: Some(8080), ..Default::default() };
    w.patch_settings().execute(request, &gui()).await.unwrap();

    assert_eq!(w.web.last().unwrap().listen_ports(), vec![8080]);
    assert_eq!(w.store_reads(), (1, 1, 1));
}

#[tokio::test]
async fn a_project_removal_syncs_without_its_worktree_environments() {
    let w = World::with_basics();
    let shop = w.project("shop", "C:\\sites\\shop", true);
    w.project("blog", "C:\\sites\\blog", true);
    w.hosts_in_sync();
    w.git.linked("C:\\wt\\fix-1", "C:\\sites\\shop", Some("fix-1"));
    w.fs.add("C:\\wt\\fix-1");
    let request = WorktreeUpRequest { path: "C:\\wt\\fix-1".into(), project_id: None, database: DatabaseModeDto::Shared };
    w.worktree_up().execute(request, &gui(), &RecordingProgress::default(), &CancelFlag::new()).await.unwrap();
    assert_eq!(served(&w), ["blog", "shop", "shop@fix-1"]);
    w.reset_store_reads();

    w.delete_project_with(w.env.clone()).execute(shop.id.as_str(), &gui()).await.unwrap();

    assert_eq!(served(&w), ["blog"]);
    assert_eq!(w.store_reads(), (1, 1, 1));
}

#[tokio::test]
async fn a_snapshot_overtaken_by_a_later_commit_is_read_again() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    w.project("blog", "C:\\sites\\blog", false);
    w.hosts_in_sync();
    let writer = w.project_writer();
    let activate = |project: &mut lampo_domain::project::Project, _: &SiteSnapshot| {
        project.set_active(true);
        Ok(true)
    };
    let first = writer.modify("shop", activate).await.unwrap();
    let second = writer.modify("blog", activate).await.unwrap();
    w.reset_store_reads();

    // The first commit's sync runs last: applying its snapshot would stop serving `blog`.
    w.env.sync_sites(second.sites, &gui()).await.unwrap();
    assert_eq!(w.store_reads(), (0, 0, 0), "the newest snapshot is used as is");
    w.env.sync_sites(first.sites, &gui()).await.unwrap();

    assert_eq!(served(&w), ["blog", "shop"]);
    assert_eq!(w.store_reads(), (1, 1, 1), "the overtaken snapshot was read again");
}

#[tokio::test]
async fn a_registry_write_without_a_snapshot_overtakes_the_earlier_ones() {
    let w = World::with_basics();
    let sites = w.allocator.reserve(|snap| Ok(snap.unchanged())).await.unwrap();
    assert!(sites.is_current());

    w.allocator.write(|| Ok(())).await.unwrap();

    assert!(!sites.is_current(), "a worktree environment removed meanwhile must not come back");
}

#[tokio::test]
async fn a_failed_sync_is_still_reported() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    let sync = Arc::new(CountingSync::default());
    *sync.fail_with.lock().unwrap() = Some(crate::error::AppError::internal("apache refused"));

    let err = SetProjectActive::new(w.project_writer_with(sync.clone())).execute(activate("shop"), &gui()).await.unwrap_err();

    assert_eq!(err.message, "apache refused");
    assert_eq!(sync.calls.load(Ordering::SeqCst), 1);
}
