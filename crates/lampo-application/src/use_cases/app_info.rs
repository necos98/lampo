use lampo_domain::paths::LampoPaths;

use crate::control::PROTOCOL_VERSION;
use crate::dto::AppInfoDto;

/// Static information about the running core. Implemented for real: the GUI shell uses it.
pub struct AppInfoQuery {
    version: String,
    paths: LampoPaths,
}

impl AppInfoQuery {
    pub fn new(version: impl Into<String>, paths: LampoPaths) -> Self {
        Self { version: version.into(), paths }
    }

    pub fn execute(&self) -> AppInfoDto {
        AppInfoDto {
            version: self.version.clone(),
            config_dir: self.paths.config_root.display().to_string(),
            data_dir: self.paths.data_root.display().to_string(),
            shim_dir: self.paths.shim_dir().display().to_string(),
            control_protocol_version: PROTOCOL_VERSION,
        }
    }
}
