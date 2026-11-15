//! The single write path of the project registry. Every use case that creates, changes or removes
//! a project goes through [`ProjectWriter`]: under the shared [`LoopbackPortAllocator`] lock,
//! after the [`ProjectEnvironmentPolicy`] and `project::check_conflicts` against every project
//! *and every worktree environment*. Its fields are private, so no rule can be skipped by saving
//! through the repository directly.

use std::sync::Arc;

use lampo_domain::events::DomainEvent;
use lampo_domain::ports::{EventPublisher, ProjectRepository};
use lampo_domain::project::{Project, ProjectId, check_conflicts};

use super::allocation::{LoopbackPortAllocator, SiteSnapshot};
use super::environment::EnvironmentSync;
use super::project_policy::ProjectEnvironmentPolicy;
use super::projects::ProjectQueries;
use super::service_catalog::ServiceCatalog;
use super::support::load_project;
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
    pub(crate) async fn insert(&self, build: impl FnOnce(&SiteSnapshot) -> AppResult<Project>) -> AppResult<Project> {
        let project = self.allocator.reserve(|snap| self.commit_new(snap, build(snap)?)).await?;
        self.changed(&project.id);
        Ok(project)
    }

    /// Like [`Self::insert`], also handing `build` a free loopback port picked under the same lock.
    pub(crate) async fn insert_with_free_port(&self, build: impl FnOnce(&SiteSnapshot, u16) -> AppResult<Project>) -> AppResult<Project> {
        let project = self.allocator.allocate(|port, snap| self.commit_new(snap, build(snap, port)?)).await?;
        self.changed(&project.id);
        Ok(project)
    }

    /// Loads `project_id`, applies `change` and, if it reports a change, saves the result, all
    /// under the allocator lock (no lost update, no conflict slipping in).
    pub(crate) async fn modify(&self, project_id: &str, change: impl FnOnce(&mut Project, &SiteSnapshot) -> AppResult<bool>) -> AppResult<Project> {
        self.allocator.reserve(|snap| self.commit_change(project_id, snap, change)).await
    }

    /// Like [`Self::modify`], also handing `change` a free loopback port picked under the same
    /// lock (it may leave it unused).
    pub(crate) async fn modify_with_free_port(&self, project_id: &str, change: impl FnOnce(&mut Project, &SiteSnapshot, u16) -> AppResult<bool>) -> AppResult<Project> {
        self.allocator.allocate(|port, snap| self.commit_change(project_id, snap, |project, snap| change(project, snap, port))).await
    }

    /// Under the allocator lock: loads `project_id`, lets `check` refuse (nothing removed),
    /// deletes the project, then runs `dependents` (its worktree environments…) on it. The
    /// project goes first: a failure there leaves everything in place, and a concurrent
    /// [`Self::modify`] either ran before or finds it gone, so it never brings it back. Publishes
    /// `ProjectsChanged`.
    pub(crate) async fn remove<T>(&self, project_id: &str, check: impl FnOnce(&Project) -> AppResult<()>, dependents: impl FnOnce(&Project, &SiteSnapshot) -> T) -> AppResult<(Project, T)> {
        let removed = self
            .allocator
            .reserve(|snap| {
                let project = self.load(project_id)?;
                check(&project)?;
                self.projects.delete(&project.id)?;
                let left = dependents(&project, snap);
                Ok((project, left))
            })
            .await?;
        self.changed(&removed.0.id);
        Ok(removed)
    }

    /// Syncs the environment to the saved configuration.
    pub(crate) async fn sync(&self, ctx: &CallContext) -> AppResult<()> {
        self.env.sync(ctx).await?;
        Ok(())
    }

    /// Syncs, then returns the full view of `project`.
    pub(crate) async fn finish(&self, project: &Project, ctx: &CallContext) -> AppResult<ProjectDto> {
        self.sync(ctx).await?;
        self.queries.get(project.id.as_str())
    }

    fn commit_new(&self, snap: &SiteSnapshot, project: Project) -> AppResult<Project> {
        self.save_checked(snap, &project, true)?;
        Ok(project)
    }

    fn commit_change(&self, project_id: &str, snap: &SiteSnapshot, change: impl FnOnce(&mut Project, &SiteSnapshot) -> AppResult<bool>) -> AppResult<Project> {
        let mut project = self.load(project_id)?;
        if change(&mut project, snap)? {
            self.save_checked(snap, &project, false)?;
            self.changed(&project.id);
        }
        Ok(project)
    }

    /// Validates `candidate` (the environment policy against the stored version, its conflicts
    /// with every project and worktree environment) and saves it. Only inside an allocator
    /// commit (`snap` comes from it).
    fn save_checked(&self, snap: &SiteSnapshot, candidate: &Project, is_new: bool) -> AppResult<()> {
        let stored = snap.projects.iter().filter(|_| !is_new).find(|p| p.id == candidate.id);
        self.policy.check(stored.map(|p| &p.environment), &candidate.environment, &snap.settings)?;
        check_conflicts(&snap.projects, &snap.worktrees, candidate, is_new, &snap.settings)?;
        self.projects.save(candidate)?;
        Ok(())
    }

    fn changed(&self, id: &ProjectId) {
        self.events.publish(DomainEvent::ProjectsChanged { changed: vec![id.clone()] });
    }
}
