//! The synchronous store reads of async use cases let the runtime's worker go
//! ([`crate::blocking`]): a read that waits on a file Windows holds must not stall the other
//! tasks of the app (GUI commands, control-pipe requests, service supervision). On the CLI's
//! current-thread runtime the same use cases run the reads inline (the other test modules).

use std::future::Future;
use std::panic;
use std::sync::Arc;
use std::time::Duration;

use lampo_domain::mysql::MysqlSeries;
use lampo_domain::progress::CancelFlag;

use super::cli;
use crate::dto::*;
use crate::testing::*;
use crate::use_cases::*;

/// How long the operation may take to reach the held read.
const ENTER_LIMIT: Duration = Duration::from_secs(5);

/// Runs `op` as a task on the only worker of a multi-threaded runtime, waits until it blocks in
/// the read `hold` was armed on, then releases the read from another task. That task needs the
/// worker: it runs only if the blocked read handed the worker over (or ran off it), otherwise
/// the held read gives up and panics. Returns what `op` returns.
pub(super) fn completes_while_a_read_is_held<T: Send + 'static>(hold: ReadHold, op: impl Future<Output = T> + Send + 'static) -> T {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    runtime.block_on(async move {
        let task = tokio::spawn(op);
        tokio::time::timeout(ENTER_LIMIT, hold.entered).await.expect("the operation never reached the held read").unwrap();
        let release = hold.release;
        // An error means the read already gave up: the operation reports it.
        tokio::spawn(async move { release.send(()) });
        match task.await {
            Ok(result) => result,
            Err(e) => panic::resume_unwind(e.into_panic()),
        }
    })
}

fn up_request(path: &str) -> WorktreeUpRequest {
    WorktreeUpRequest { path: path.into(), project_id: None, database: DatabaseModeDto::Shared }
}

/// `shop` at `C:\sites\shop`, with a linked git worktree at `C:\wt\fix-1`.
fn world_with_a_worktree_folder() -> World {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", true);
    w.hosts_in_sync();
    w.git.linked("C:\\wt\\fix-1", "C:\\sites\\shop", Some("fix-1"));
    w.fs.add("C:\\wt\\fix-1");
    w
}

#[test]
fn project_detection_lets_the_worker_go() {
    let w = World::with_basics();
    w.fs.add("C:\\sites\\shop");
    let detect = Arc::new(DetectProject::new(vec![], w.projects.clone(), w.fs.clone()));
    let create = CreateProject::new(w.project_writer(), w.presets(), detect, w.fs.clone(), w.database_service());
    let hold = w.projects.2.hold_next();

    let request = CreateProjectRequest { root: "C:\\sites\\shop".into(), addressing: AddressingDto::NoElevation, ..Default::default() };
    let created = completes_while_a_read_is_held(hold, async move { create.execute(request, &cli()).await });

    assert_eq!(created.unwrap().project.id, "shop");
}

#[test]
fn worktree_placement_lets_the_worker_go() {
    let w = world_with_a_worktree_folder();
    let up = w.worktree_up();
    let hold = w.projects.2.hold_next();

    let result = completes_while_a_read_is_held(hold, async move { up.execute(up_request("C:\\wt\\fix-1"), &cli(), &RecordingProgress::default(), &CancelFlag::new()).await });

    assert!(result.unwrap().created);
}

#[test]
fn the_existing_environment_lookup_lets_the_worker_go() {
    let w = world_with_a_worktree_folder();
    let hold = w.worktrees.1.hold_next();
    let up = w.worktree_up();

    let result = completes_while_a_read_is_held(hold, async move { up.execute(up_request("C:\\wt\\fix-1"), &cli(), &RecordingProgress::default(), &CancelFlag::new()).await });

    assert!(result.unwrap().created);
}

#[test]
fn worktree_down_lookup_lets_the_worker_go() {
    let w = world_with_a_worktree_folder();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(w.worktree_up().execute(up_request("C:\\wt\\fix-1"), &cli(), &RecordingProgress::default(), &CancelFlag::new())).unwrap();
    let down = w.worktree_down();
    let hold = w.worktrees.1.hold_next();

    let request = WorktreeDownRequest { target: "shop@fix-1".into(), drop_database: false };
    let removed = completes_while_a_read_is_held(hold, async move { down.execute(request, &cli()).await });

    assert_eq!(removed.unwrap().site, "shop@fix-1");
}

#[test]
fn the_launch_of_a_service_start_lets_the_worker_go() {
    let w = World::with_basics();
    let mysql = MysqlSeries::DEFAULT.service_id();
    let env = w.env.clone();
    let hold = w.settings.2.hold_next();

    let started = completes_while_a_read_is_held(hold, async move { env.ensure(&mysql).await });

    assert!(started.unwrap());
}
