use serde::{Deserialize, Serialize};

use super::project::ProjectEnvironmentDto;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetDto {
    pub id: String,
    /// English fallback; the GUI localises built-ins by `id`.
    pub name: String,
    pub description: String,
    pub builtin: bool,
    pub environment: ProjectEnvironmentDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavePresetRequest {
    /// Project whose environment is copied.
    pub project_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}
