use std::sync::Arc;

use lampo_domain::php::PhpMinor;
use lampo_domain::ports::{CustomPresetRepository, InstalledPackageRegistry, ProjectRepository, SettingsRepository};
use lampo_domain::preset::{Preset, PresetContext, PresetDefinition, PresetId};
use lampo_domain::project::DisplayName;
use lampo_domain::DomainError;

use super::support::{installed_php_minors, load_project};
use crate::dto::{PresetDto, SavePresetRequest};
use crate::error::{AppError, AppResult};
use crate::mapping::environment_to_dto;

pub(crate) fn preset_to_dto(p: &Preset) -> PresetDto {
    PresetDto { id: p.id.to_string(), name: p.name.to_string(), description: p.description.clone(), builtin: p.builtin, environment: environment_to_dto(&p.environment) }
}

fn is_builtin(builtins: &[Arc<dyn PresetDefinition>], id: &PresetId) -> bool {
    builtins.iter().any(|d| &d.id() == id)
}

/// Built-in (from [`PresetDefinition`]s) and custom presets.
pub struct PresetQueries {
    builtins: Vec<Arc<dyn PresetDefinition>>,
    custom: Arc<dyn CustomPresetRepository>,
    settings: Arc<dyn SettingsRepository>,
    installed: Arc<dyn InstalledPackageRegistry>,
}

impl PresetQueries {
    /// `builtins` is `lampo_domain::preset::builtin::all()` in production; the same list goes to
    /// [`SavePreset`] and [`DeletePreset`].
    pub fn new(
        builtins: Vec<Arc<dyn PresetDefinition>>,
        custom: Arc<dyn CustomPresetRepository>,
        settings: Arc<dyn SettingsRepository>,
        installed: Arc<dyn InstalledPackageRegistry>,
    ) -> Self {
        Self { builtins, custom, settings, installed }
    }

    fn context(&self) -> AppResult<PresetContext> {
        let latest_php = installed_php_minors(&self.installed.list()?).last().copied().unwrap_or(PhpMinor::LATEST_KNOWN);
        Ok(PresetContext { latest_php, default_mysql: self.settings.load()?.default_mysql() })
    }

    fn all(&self) -> AppResult<Vec<Preset>> {
        let ctx = self.context()?;
        let mut out: Vec<Preset> = self.builtins.iter().map(|d| d.build(&ctx)).collect();
        let mut custom: Vec<Preset> = self.custom.list()?.into_iter().filter(|p| !is_builtin(&self.builtins, &p.id)).collect();
        custom.sort_by_key(|p| p.name.as_str().to_lowercase());
        out.extend(custom);
        Ok(out)
    }

    /// Built-ins first (definition order), then custom presets by name.
    pub fn list(&self) -> AppResult<Vec<PresetDto>> {
        Ok(self.all()?.iter().map(preset_to_dto).collect())
    }

    /// Domain preset by id, for other use cases. `PresetContext.latest_php` = newest installed
    /// PHP minor, else `PhpMinor::LATEST_KNOWN`.
    pub fn find(&self, preset_id: &str) -> AppResult<Preset> {
        let id = PresetId::new(preset_id.trim())?;
        self.all()?.into_iter().find(|p| p.id == id).ok_or_else(|| AppError::not_found("preset", &id))
    }
}

pub struct SavePreset {
    projects: Arc<dyn ProjectRepository>,
    custom: Arc<dyn CustomPresetRepository>,
    builtins: Vec<Arc<dyn PresetDefinition>>,
}

impl SavePreset {
    /// `builtins`: the same list given to [`PresetQueries`] (their ids are read-only).
    pub fn new(projects: Arc<dyn ProjectRepository>, custom: Arc<dyn CustomPresetRepository>, builtins: Vec<Arc<dyn PresetDefinition>>) -> Self {
        Self { projects, custom, builtins }
    }

    /// Snapshot of the project's environment as a new custom preset (`Conflict` on id clash
    /// with a built-in). Saving again under the name of an existing custom preset replaces it.
    pub fn execute(&self, request: SavePresetRequest) -> AppResult<PresetDto> {
        let project = load_project(self.projects.as_ref(), &request.project_id)?;
        let name = DisplayName::new(&request.name)?;
        let id = PresetId::from_name(name.as_str());
        if is_builtin(&self.builtins, &id) {
            return Err(DomainError::BuiltinPresetIsReadOnly(id.to_string()).into());
        }
        let preset = Preset::from_project(id, name, request.description.trim(), &project);
        self.custom.save(&preset)?;
        Ok(preset_to_dto(&preset))
    }
}

pub struct DeletePreset {
    custom: Arc<dyn CustomPresetRepository>,
    builtins: Vec<Arc<dyn PresetDefinition>>,
}

impl DeletePreset {
    /// `builtins`: the same list given to [`PresetQueries`] (they cannot be deleted).
    pub fn new(custom: Arc<dyn CustomPresetRepository>, builtins: Vec<Arc<dyn PresetDefinition>>) -> Self {
        Self { custom, builtins }
    }

    /// `Conflict` for built-ins, `NotFound` for unknown ids.
    pub fn execute(&self, preset_id: &str) -> AppResult<()> {
        let id = PresetId::new(preset_id.trim())?;
        if is_builtin(&self.builtins, &id) {
            return Err(DomainError::BuiltinPresetIsReadOnly(id.to_string()).into());
        }
        let preset = self.custom.list()?.into_iter().find(|p| p.id == id).ok_or_else(|| AppError::not_found("preset", &id))?;
        preset.ensure_editable()?;
        Ok(self.custom.delete(&id)?)
    }
}
