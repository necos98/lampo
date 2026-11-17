//! The registry (projects, worktree environments, settings) as loaded under the allocator lock,
//! and how a commit hands it on: the use case that saved something describes its save on the
//! snapshot ([`SiteSnapshot::with_project`]…), and the sync after it reuses that instead of
//! reading every file again.
//!
//! Reuse is only safe while the snapshot is the newest state of the registry. Every commit under
//! the allocator lock takes a new [`RegistryRevision`] number before it writes; a snapshot that
//! describes a commit carries that commit's number, and [`SiteSnapshot::is_current`] holds only
//! while no later commit has started. Anything else (a snapshot nobody marked as describing its
//! commit, one overtaken by another commit) is read again by the sync.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use lampo_domain::ports::ProjectListing;
use lampo_domain::project::{Project, ProjectId, unique_project_id};
use lampo_domain::settings::Settings;
use lampo_domain::tcp_port::Port;
use lampo_domain::worktree::WorktreeEnvironment;

/// Numbers the commits to the registry, in the order they take the allocator lock. Shared by an
/// allocator and the snapshots it hands out.
#[derive(Debug, Default)]
pub(crate) struct RegistryRevision(AtomicU64);

impl RegistryRevision {
    /// Starts a commit: its number, greater than every earlier one.
    pub(crate) fn begin_commit(&self) -> u64 {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn latest(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Which state of the registry a snapshot shows.
#[derive(Debug, Clone)]
struct Stamp {
    revision: Arc<RegistryRevision>,
    /// The commit during which the snapshot was loaded.
    commit: u64,
    /// The snapshot shows the registry after that commit (set by the use case that wrote it).
    describes_commit: bool,
}

/// The registry as seen under the allocation lock: what a commit validates against.
#[derive(Debug, Clone)]
pub struct SiteSnapshot {
    /// Sorted by id, as the repository lists them.
    pub projects: Vec<Project>,
    /// Loopback ports named by project files that do not load: never handed out, so a repaired
    /// project does not find its listener taken.
    pub reserved_ports: BTreeSet<Port>,
    /// Ids of the project files that do not load: never given to a new project, whose save would
    /// replace the file and inherit its worktree environments.
    pub reserved_ids: BTreeSet<ProjectId>,
    /// Sorted by parent and slug, as the repository lists them.
    pub worktrees: Vec<WorktreeEnvironment>,
    pub settings: Settings,
    stamp: Stamp,
}

impl SiteSnapshot {
    /// Loaded during `commit` of `revision`, before anything was written.
    pub(crate) fn loaded(listing: ProjectListing, worktrees: Vec<WorktreeEnvironment>, settings: Settings, revision: Arc<RegistryRevision>, commit: u64) -> Self {
        let reserved_ports = listing.unreadable.iter().flat_map(|u| u.loopback_ports.iter().copied()).collect();
        let reserved_ids = listing.unreadable_ids();
        Self { projects: listing.projects, reserved_ports, reserved_ids, worktrees, settings, stamp: Stamp { revision, commit, describes_commit: false } }
    }

    /// The first free id derived from `base`, for a new project: neither a project's nor a
    /// [`Self::reserved_ids`] one.
    pub(crate) fn unique_project_id(&self, base: &ProjectId) -> ProjectId {
        unique_project_id(&self.projects, &self.reserved_ids, base)
    }

    /// The registry after a commit that wrote nothing.
    pub(crate) fn unchanged(&self) -> Self {
        let mut after = self.clone();
        after.stamp.describes_commit = true;
        after
    }

    /// The registry after a commit that saved `project` (new or replacing the one with its id).
    pub(crate) fn with_project(&self, project: &Project) -> Self {
        let mut after = self.unchanged();
        match after.projects.binary_search_by(|p| p.id.cmp(&project.id)) {
            Ok(i) => after.projects[i] = project.clone(),
            Err(i) => after.projects.insert(i, project.clone()),
        }
        after
    }

    /// The registry after a commit that deleted the project `id` (its worktree environments stay
    /// until [`Self::forget_worktrees_of`]).
    pub(crate) fn without_project(&self, id: &ProjectId) -> Self {
        let mut after = self.unchanged();
        after.projects.retain(|p| &p.id != id);
        after
    }

    /// Also deleted, in the same commit: every worktree environment of `parent`.
    pub(crate) fn forget_worktrees_of(&mut self, parent: &ProjectId) {
        self.worktrees.retain(|w| &w.parent != parent);
    }

    /// The registry after a commit that saved `environment` (new or replacing its slot).
    pub(crate) fn with_worktree(&self, environment: &WorktreeEnvironment) -> Self {
        let mut after = self.unchanged();
        let wanted = (&environment.parent, environment.slug.as_str());
        match after.worktrees.iter().position(|w| (&w.parent, w.slug.as_str()) == wanted) {
            Some(i) => after.worktrees[i] = environment.clone(),
            None => {
                let at = after.worktrees.partition_point(|w| (&w.parent, w.slug.as_str()) < wanted);
                after.worktrees.insert(at, environment.clone());
            }
        }
        after
    }

    /// The registry after a commit that saved `settings`.
    pub(crate) fn with_settings(&self, settings: Settings) -> Self {
        let mut after = self.unchanged();
        after.settings = settings;
        after
    }

    /// Shows the registry after its commit, and no later commit has started: what a sync may use
    /// instead of reading the files again.
    pub(crate) fn is_current(&self) -> bool {
        self.stamp.describes_commit && self.stamp.revision.latest() == self.stamp.commit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::World;

    fn loaded(revision: &Arc<RegistryRevision>, projects: Vec<Project>) -> SiteSnapshot {
        let commit = revision.begin_commit();
        SiteSnapshot::loaded(ProjectListing { projects, unreadable: Vec::new() }, Vec::new(), Settings::default(), revision.clone(), commit)
    }

    #[test]
    fn only_a_snapshot_describing_the_newest_commit_is_current() {
        let revision = Arc::new(RegistryRevision::default());
        let before = loaded(&revision, Vec::new());
        assert!(!before.is_current(), "loaded before the commit wrote");
        let after = before.unchanged();
        assert!(after.is_current());
        let _next = loaded(&revision, Vec::new());
        assert!(!after.is_current(), "overtaken by a later commit");
    }

    #[test]
    fn saved_projects_keep_the_repository_order() {
        let w = World::new();
        let (b, a, c) = (w.project("b", "C:\\b", true), w.project("a", "C:\\a", true), w.project("c", "C:\\c", true));
        let snap = loaded(&Arc::default(), vec![a.clone(), c.clone()]);
        let ids = |s: &SiteSnapshot| s.projects.iter().map(|p| p.id.to_string()).collect::<Vec<_>>();
        assert_eq!(ids(&snap.with_project(&b)), ["a", "b", "c"]);
        let mut renamed = a.clone();
        renamed.active = false;
        let replaced = snap.with_project(&renamed);
        assert_eq!(ids(&replaced), ["a", "c"]);
        assert!(!replaced.projects[0].active);
        assert_eq!(ids(&snap.without_project(&c.id)), ["a"]);
    }
}
