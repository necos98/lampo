use std::sync::Arc;

use lampo_domain::ports::{EventPublisher, SettingsRepository};

use super::environment::EnvironmentSync;
use crate::context::CallContext;
use crate::dto::{ServiceKindDto, SettingsDto};
use crate::error::AppResult;
use crate::mapping::{service_kinds_to_dto, settings_from_dto, settings_to_dto};
use super::service_catalog::ServiceCatalog;
use lampo_domain::events::DomainEvent;

pub struct SettingsService {
    repo: Arc<dyn SettingsRepository>,
    env: Arc<dyn EnvironmentSync>,
    events: Arc<dyn EventPublisher>,
    services: Arc<ServiceCatalog>,
}

impl SettingsService {
    pub fn new(repo: Arc<dyn SettingsRepository>, env: Arc<dyn EnvironmentSync>, events: Arc<dyn EventPublisher>, services: Arc<ServiceCatalog>) -> Self {
        Self { repo, env, events, services }
    }

    pub fn get(&self) -> AppResult<SettingsDto> {
        Ok(settings_to_dto(&self.repo.load()?, &self.services))
    }

    /// The service kinds a project may use, with their configured versions.
    pub fn service_kinds(&self) -> AppResult<Vec<ServiceKindDto>> {
        Ok(service_kinds_to_dto(&self.services, &self.repo.load()?))
    }

    /// Validates (`settings_from_dto`), saves, publishes `SettingsChanged`, syncs (ports/TLD
    /// changes regenerate config and may move services).
    pub async fn update(&self, settings: SettingsDto, ctx: &CallContext) -> AppResult<SettingsDto> {
        let current = self.repo.load()?;
        let next = settings_from_dto(&settings, &current)?;
        if next != current {
            self.repo.save(&next)?;
            self.events.publish(DomainEvent::SettingsChanged);
        }
        self.env.sync(ctx).await?;
        Ok(settings_to_dto(&next, &self.services))
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
