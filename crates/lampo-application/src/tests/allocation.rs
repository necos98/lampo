//! M4: one loopback-port allocator, shared by project creation and worktree environments, makes
//! "allocate and save" atomic across concurrent callers (GUI + several agents on the pipe).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use lampo_domain::address::SiteAddress;
use lampo_domain::ports::{ProjectRepository, WorktreeRepository};
use lampo_domain::progress::CancelFlag;
use lampo_domain::project::{ProjectId, ProjectRoot, check_conflicts};
use lampo_domain::worktree::{DatabaseMode, Occupancy, WorktreeEnvironment};

use super::cli;
use crate::dto::*;
use crate::error::AppResult;
use crate::testing::*;
use crate::use_cases::*;

/// Every loopback port in use, failing on a duplicate.
fn assert_distinct_ports(w: &World) -> BTreeSet<u16> {
    let mut seen = BTreeSet::new();
    for p in w.projects.list().unwrap() {
        for port in p.addresses().iter().filter_map(SiteAddress::loopback) {
            assert!(seen.insert(port), "port {port} assigned twice (project {})", p.id);
        }
    }
    for wt in w.worktrees.list_all().unwrap() {
        assert!(seen.insert(wt.loopback_port), "port {} assigned twice (worktree {})", wt.loopback_port, wt.site_id());
    }
    seen
}

/// A project whose file does not load is hidden, not gone: the user may repair it, so its
/// loopback port is never handed to another site.
#[tokio::test]
async fn ports_named_by_unreadable_project_files_are_not_allocated() {
    let w = World::with_basics();
    let first = w.allocator.peek().await.unwrap();
    w.unreadable_project("C:\\Lampo\\projects\\blog.toml", &[first]);
    let next = w.allocator.peek().await.unwrap();
    assert_ne!(next, first);
    assert_eq!(w.allocator.allocate(|port, snap| Ok((port, snap.reserved_ports.clone()))).await.unwrap(), (next, BTreeSet::from([first])));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_allocations_never_hand_out_the_same_port() {
    let w = World::with_basics();
    w.project("shop", "C:\\sites\\shop", false);
    let parent = w.projects.list().unwrap().remove(0);
    w.ports.slow(3);
    w.ports.busy(8101, "node.exe");

    let mut tasks = Vec::new();
    for n in 0..16 {
        let allocator = w.allocator.clone();
        let projects = w.projects.clone();
        let worktrees = w.worktrees.clone();
        let parent = parent.clone();
        tasks.push(tokio::spawn(async move {
            allocator
                .allocate(|port, snap| {
                    // Project and worktree saves interleave, as `lampo project create` and
                    // `lampo worktree up` would.
                    if n % 2 == 0 {
                        let mut p = parent.clone();
                        p.id = ProjectId::new(format!("p{n}")).unwrap();
                        p.root = ProjectRoot::new(format!("C:\\sites\\p{n}")).unwrap();
                        p.set_addresses(vec![SiteAddress::localhost(p.id.as_str())?, SiteAddress::LoopbackPort(port)])?;
                        check_conflicts(&snap.projects, &snap.worktrees, &p, true, &snap.settings)?;
                        projects.save(&p)?;
                    } else {
                        let root = ProjectRoot::new(format!("D:\\wt\\{n}")).unwrap();
                        let taken = Occupancy { projects: &snap.projects, worktrees: &snap.worktrees, ..Default::default() };
                        let env = WorktreeEnvironment::derive(&parent, &taken, root, Some(format!("b{n}")), port, &DatabaseMode::Shared, 0)?;
                        worktrees.save(&env)?;
                    }
                    Ok(port)
                })
                .await
        }));
    }
    let mut ports = BTreeSet::new();
    for t in tasks {
        assert!(ports.insert(t.await.unwrap().unwrap()));
    }
    assert_eq!(assert_distinct_ports(&w), ports);
    assert!(!ports.contains(&8101), "busy on the OS");
    assert_eq!(ports.len(), 16);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn project_create_and_worktree_up_racing_get_distinct_ports() {
    let w = Arc::new(World::with_basics());
    w.project("shop", "C:\\sites\\shop", false);
    w.hosts_in_sync();
    w.ports.slow(2);
    for n in 0..6 {
        w.fs.add(&format!("C:\\sites\\new{n}"));
        w.fs.add(&format!("D:\\wt\\{n}"));
        w.git.linked(&format!("D:\\wt\\{n}"), "C:\\sites\\shop", Some(&format!("feature-{n}")));
    }
    let detect = Arc::new(DetectProject::new(vec![], w.projects.clone(), w.fs.clone()));
    let create = Arc::new(CreateProject::new(w.project_writer(), w.presets(), detect, w.fs.clone(), w.database_service()));
    let up = Arc::new(w.worktree_up());

    let mut tasks = Vec::new();
    for n in 0..6 {
        let create = create.clone();
        tasks.push(tokio::spawn(async move {
            let request = CreateProjectRequest { root: format!("C:\\sites\\new{n}"), addressing: AddressingDto::NoElevation, ..Default::default() };
            create.execute(request, &cli()).await.map(|_| ())
        }));
        let up = up.clone();
        tasks.push(tokio::spawn(async move {
            let request = WorktreeUpRequest { path: format!("D:\\wt\\{n}"), project_id: None, database: DatabaseModeDto::Shared };
            up.execute(request, &cli(), &RecordingProgress::default(), &CancelFlag::new()).await.map(|_| ())
        }));
    }
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    assert_eq!(assert_distinct_ports(&w).len(), 12);
}

/// How long a save that needs the allocator lock must still be pending while another commit
/// holds it. A save that skips the lock finishes long before (the fakes are in memory).
const LOCK_WAIT: Duration = Duration::from_millis(100);

/// Holds the allocator lock in another commit, starts `op`, checks that `op` is still waiting
/// for the lock, then runs `locked_edit` (the other commit's save, under the lock) and releases
/// the lock. Returns what `op` returns once it ran after `locked_edit`.
pub(super) async fn after_a_locked_commit<T: Send + 'static>(
    w: &World,
    locked_edit: impl FnOnce(&SiteSnapshot) -> AppResult<()> + Send + 'static,
    op: impl Future<Output = T> + Send + 'static,
) -> T {
    let (held, is_held) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let allocator = w.allocator.clone();
    let holder = tokio::spawn(async move {
        allocator
            .reserve(move |snap| {
                held.send(()).expect("the test waits for the lock to be held");
                released.recv().expect("the test releases the lock");
                locked_edit(snap)
            })
            .await
    });
    is_held.await.unwrap();
    let mut op = tokio::spawn(op);
    assert!(tokio::time::timeout(LOCK_WAIT, &mut op).await.is_err(), "the save must wait for the allocator lock");
    release.send(()).unwrap();
    holder.await.unwrap().unwrap();
    op.await.unwrap()
}
