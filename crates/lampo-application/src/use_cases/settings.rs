use std::sync::Arc;

use lampo_domain::ports::SettingsRepository;

use super::environment::EnvironmentSync;
use super::service_catalog::ServiceCatalog;
use super::settings_writer::{SettingsCommit, SettingsWriter};
use crate::context::CallContext;
use crate::dto::{ServiceKindDto, SettingsDto};
use crate::error::AppResult;
use crate::mapping::{service_kinds_to_dto, settings_from_dto, settings_to_dto};

/// The GUI settings page: reads and saves the whole [`SettingsDto`].
pub struct SettingsService {
    writer: SettingsWriter,
    env: Arc<dyn EnvironmentSync>,
}

impl SettingsService {
    pub fn new(writer: SettingsWriter, env: Arc<dyn EnvironmentSync>) -> Self {
        Self { writer, env }
    }

    pub fn get(&self) -> AppResult<SettingsDto> {
        Ok(settings_to_dto(&self.writer.load()?, &self.writer.services))
    }

    /// The service kinds a project may use, with their configured versions.
    pub fn service_kinds(&self) -> AppResult<Vec<ServiceKindDto>> {
        Ok(service_kinds_to_dto(&self.writer.services, &self.writer.load()?))
    }

    /// Converts and validates against the stored settings (`settings_from_dto`: the service
    /// instances cannot be added or removed here), then saves through [`SettingsWriter::modify`]
    /// with the same checks as `lampo settings set` (`Conflict` for a site's loopback port,
    /// `PortInUse`, `PackageMissing` for a default PHP that is not installed). Then syncs
    /// (ports/TLD changes regenerate config and may move services).
    pub async fn update(&self, settings: SettingsDto, ctx: &CallContext) -> AppResult<SettingsDto> {
        let SettingsCommit { after, sites, .. } = self.writer.modify(|current| Ok(settings_from_dto(&settings, current)?)).await?;
        self.env.sync_sites(sites, ctx).await?;
        Ok(settings_to_dto(&after, &self.writer.services))
    }
}

/// Read-only view of the settings (`lampo settings get`, file-based: runs without the app).
pub struct SettingsQuery {
    repo: Arc<dyn SettingsRepository>,
    services: Arc<ServiceCatalog>,
}

impl SettingsQuery {
    pub fn new(repo: Arc<dyn SettingsRepository>, services: Arc<ServiceCatalog>) -> Self {
        Self { repo, services }
    }

    pub fn get(&self) -> AppResult<SettingsDto> {
        Ok(settings_to_dto(&self.repo.load()?, &self.services))
    }
}
