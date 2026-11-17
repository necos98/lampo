//! The single write path of the project registry. Every use case that creates, changes or removes
//! a project goes through [`ProjectWriter`]: under the shared [`LoopbackPortAllocator`] lock,
//! after the [`ProjectEnvironmentPolicy`] and `project::check_conflicts` against every project
//! *and every worktree environment*. Its fields are private, so no rule can be skipped by saving
//! through the repository directly.

use std::sync::Arc;

use lampo_domain::events::DomainEvent;
use lampo_domain::ports::{EventPublisher, ProjectRepository};
use lampo_domain::project::{Project, ProjectId, check_conflicts, check_unreadable_id};
use lampo_domain::tcp_port::Port;

use super::allocation::{LoopbackPortAllocator, SiteSnapshot};
use super::environment::EnvironmentSync;
use super::project_policy::{ProjectEnvironmentPolicy, normalize_extensions};
use super::projects::ProjectQueries;
use super::service_catalog::ServiceCatalog;
use super::support::{load_project, parse_project_id};
use crate::context::CallContext;
use crate::dto::ProjectDto;
use crate::error::AppResult;

/// What a [`ProjectWriter`] is built from. All required; the composition roots share the
/// allocator with every other use case that takes it.
pub struct ProjectWriterDeps {
    pub projects: Arc<dyn ProjectRepository>,
    pub allocator: Arc<LoopbackPortAllocator>,
    pub env: Arc<dyn EnvironmentSync>,
    pub events: Arc<dyn EventPublisher>,
    pub queries: Arc<ProjectQueries>,
    pub policy: Arc<ProjectEnvironmentPolicy>,
}

#[derive(Clone)]
pub struct ProjectWriter {
    projects: Arc<dyn ProjectRepository>,
    allocator: Arc<LoopbackPortAllocator>,
    env: Arc<dyn EnvironmentSync>,
    events: Arc<dyn EventPublisher>,
    queries: Arc<ProjectQueries>,
    policy: Arc<ProjectEnvironmentPolicy>,
}

impl ProjectWriter {
    pub fn new(deps: ProjectWriterDeps) -> Self {
        let ProjectWriterDeps { projects, allocator, env, events, queries, policy } = deps;
        Self { projects, allocator, env, events, queries, policy }
    }

    /// The registered service kinds: a project may only use these.
    pub(crate) fn services(&self) -> &Arc<ServiceCatalog> {
        self.policy.services()
    }

    /// The stored project (`NotFound` if unknown).
    pub(crate) fn load(&self, project_id: &str) -> AppResult<Project> {
        load_project(self.projects.as_ref(), project_id)
    }

    /// Saves the new project `build` makes from the registry seen under the allocator lock, then
    /// publishes `ProjectsChanged`. `build` may refuse (nothing is saved).
    pub(crate) async fn insert(&self, build: impl FnOnce(&SiteSnapshot) -> AppResult<Project>) -> AppResult<SavedProject> {
        let saved = self.allocator.reserve(|snap| self.commit_new(snap, build(snap)?)).await?;
        self.changed(&saved.project.id);
        Ok(saved)
    }

    /// Like [`Self::insert`], also handing `build` a free loopback port picked under the same lock.
    pub(crate) async fn insert_with_free_port(&self, build: impl FnOnce(&SiteSnapshot, Port) -> AppResult<Project>) -> AppResult<SavedProject> {
        let saved = self.allocator.allocate(|port, snap| self.commit_new(snap, build(snap, port)?)).await?;
        self.changed(&saved.project.id);
        Ok(saved)
    }

    /// Loads `project_id`, applies `change` and, if it reports a change, saves the result, all
    /// under the allocator lock (no lost update, no conflict slipping in).
    pub(crate) async fn modify(&self, project_id: &str, change: impl FnOnce(&mut Project, &SiteSnapshot) -> AppResult<bool>) -> AppResult<SavedProject> {
        self.allocator.reserve(|snap| self.commit_change(project_id, snap, change)).await
    }

    /// Like [`Self::modify`], also handing `change` a free loopback port picked under the same
    /// lock (it may leave it unused).
    pub(crate) async fn modify_with_free_port(&self, project_id: &str, change: impl FnOnce(&mut Project, &SiteSnapshot, Port) -> AppResult<bool>) -> AppResult<SavedProject> {
        self.allocator.allocate(|port, snap| self.commit_change(project_id, snap, |project, snap| change(project, snap, port))).await
    }

    /// Under the allocator lock: loads `project_id`, lets `check` refuse (nothing removed),
    /// deletes the project, then runs `dependents` (its worktree environments…) on it and on the
    /// registry without it, which `dependents` updates with what it removes too. The project goes
    /// first: a failure there leaves everything in place, and a concurrent [`Self::modify`] either
    /// ran before or finds it gone, so it never brings it back. Publishes `ProjectsChanged`.
    pub(crate) async fn remove<T>(
        &self,
        project_id: &str,
        check: impl FnOnce(&Project) -> AppResult<()>,
        dependents: impl FnOnce(&Project, &mut SiteSnapshot) -> T,
    ) -> AppResult<RemovedProject<T>> {
        let removed = self
            .allocator
            .reserve(|snap| {
                let project = self.stored(project_id, snap)?;
                check(&project)?;
                self.projects.delete(&project.id)?;
                let mut sites = snap.without_project(&project.id);
                let dependents = dependents(&project, &mut sites);
                Ok(RemovedProject { project, dependents, sites })
            })
            .await?;
        self.changed(&removed.project.id);
        Ok(removed)
    }

    /// Syncs the environment to the registry `sites` a commit handed on.
    pub(crate) async fn sync(&self, sites: SiteSnapshot, ctx: &CallContext) -> AppResult<()> {
        self.env.sync_sites(sites, ctx).await?;
        Ok(())
    }

    /// Syncs, then returns the full view of the saved project, both from the registry the save
    /// handed on (nothing is read again), with the policy's notices on what was saved (an
    /// extension compiled into its PHP).
    pub(crate) async fn finish(&self, saved: SavedProject, ctx: &CallContext) -> AppResult<ProjectDto> {
        let mut view = self.queries.present(&saved.project, &saved.sites);
        view.notices = self.policy.notices(&saved.project.environment);
        self.sync(saved.sites, ctx).await?;
        Ok(view)
    }

    /// `project_id` as stored: from the snapshot, else read (`NotFound`, or why its file does not
    /// load).
    fn stored(&self, project_id: &str, snap: &SiteSnapshot) -> AppResult<Project> {
        let id = parse_project_id(project_id)?;
        match snap.projects.iter().find(|p| p.id == id) {
            Some(project) => Ok(project.clone()),
            None => self.load(project_id),
        }
    }

    fn commit_new(&self, snap: &SiteSnapshot, mut project: Project) -> AppResult<SavedProject> {
        self.save_checked(snap, &mut project, true)?;
        Ok(SavedProject { sites: snap.with_project(&project), project })
    }

    fn commit_change(&self, project_id: &str, snap: &SiteSnapshot, change: impl FnOnce(&mut Project, &SiteSnapshot) -> AppResult<bool>) -> AppResult<SavedProject> {
        let mut project = self.stored(project_id, snap)?;
        if !change(&mut project, snap)? {
            return Ok(SavedProject { sites: snap.unchanged(), project });
        }
        self.save_checked(snap, &mut project, false)?;
        self.changed(&project.id);
        Ok(SavedProject { sites: snap.with_project(&project), project })
    }

    /// Names `candidate`'s extensions as its PHP does, validates it (the environment policy
    /// against the stored version, its conflicts with every project and worktree environment)
    /// and saves it. Only inside an allocator commit (`snap` comes from it).
    fn save_checked(&self, snap: &SiteSnapshot, candidate: &mut Project, is_new: bool) -> AppResult<()> {
        normalize_extensions(&mut candidate.environment);
        let stored = snap.projects.iter().filter(|_| !is_new).find(|p| p.id == candidate.id);
        self.policy.check(stored.map(|p| &p.environment), &candidate.environment, &snap.settings)?;
        check_conflicts(&snap.projects, &snap.worktrees, candidate, is_new, &snap.settings)?;
        if is_new {
            check_unreadable_id(&snap.reserved_ids, &candidate.id)?;
        }
        self.projects.save(candidate)?;
        Ok(())
    }

    fn changed(&self, id: &ProjectId) {
        self.events.publish(DomainEvent::ProjectsChanged { changed: vec![id.clone()] });
    }
}

/// A project as a commit saved it (or found it, when nothing changed), with the registry after
/// that commit: what the sync and the view that follow use instead of reading the files again.
pub(crate) struct SavedProject {
    pub project: Project,
    pub sites: SiteSnapshot,
}

/// A removed project, what its `dependents` step returned, and the registry after the removal.
pub(crate) struct RemovedProject<T> {
    pub project: Project,
    pub dependents: T,
    pub sites: SiteSnapshot,
}
