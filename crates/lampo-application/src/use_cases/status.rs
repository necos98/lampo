use std::sync::Arc;

use super::projects::ProjectQueries;
use super::services::ServiceStatusSource;
use crate::dto::StatusDto;
use crate::error::AppResult;

/// `lampo status` / dashboard header.
pub struct StatusQuery {
    projects: Arc<ProjectQueries>,
    services: Arc<dyn ServiceStatusSource>,
    app_running: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl StatusQuery {
    /// `app_running`: `|| true` inside the app; `ControlChannelClient::is_server_running` in the CLI.
    pub fn new(projects: Arc<ProjectQueries>, services: Arc<dyn ServiceStatusSource>, app_running: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self { projects, services, app_running }
    }

    pub async fn execute(&self) -> AppResult<StatusDto> {
        let listing = self.projects.summaries()?;
        let services = self.services.statuses().await?;
        Ok(StatusDto {
            app_running: (self.app_running)(),
            worktree_count: listing.projects.iter().map(|p| p.worktree_count).sum(),
            services,
            projects: listing.projects,
            unreadable_projects: listing.unreadable,
        })
    }
}
