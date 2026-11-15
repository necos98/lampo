//! Resolves the two Lampo roots from the Windows environment.

use std::ffi::OsString;
use std::path::PathBuf;

use lampo_domain::paths::LampoPaths;
use lampo_domain::{PortError, PortResult};

/// Folder name under `%APPDATA%` and `%LOCALAPPDATA%`.
pub const APP_DIR_NAME: &str = "Lampo";

/// Environment variable overriding both roots (`<dir>\config`, `<dir>\data`), for tests and
/// portable setups.
pub const HOME_OVERRIDE_VAR: &str = "LAMPO_HOME";

/// `%APPDATA%\Lampo` and `%LOCALAPPDATA%\Lampo` (or `LAMPO_HOME`). Does not create them.
pub fn resolve_lampo_paths() -> PortResult<LampoPaths> {
    resolve_with(std::env::var_os(HOME_OVERRIDE_VAR))
}

/// The `LAMPO_HOME` override, when set (non-empty). An isolated instance must not share
/// machine-wide state with the real one: composition roots use it to pick a separate control
/// pipe and to leave the user PATH alone.
pub fn home_override() -> Option<PathBuf> {
    std::env::var_os(HOME_OVERRIDE_VAR).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// [`resolve_lampo_paths`] with the override passed in (pure, testable without touching the
/// process environment). An empty override counts as unset.
pub fn resolve_with(home_override: Option<OsString>) -> PortResult<LampoPaths> {
    if let Some(home) = home_override.filter(|v| !v.is_empty()) {
        let home = PathBuf::from(home);
        return Ok(LampoPaths::new(home.join("config"), home.join("data")));
    }
    let config = dirs::config_dir().ok_or_else(|| PortError::io("locating %APPDATA%", "not available"))?;
    let data = dirs::data_local_dir().ok_or_else(|| PortError::io("locating %LOCALAPPDATA%", "not available"))?;
    Ok(LampoPaths::new(config.join(APP_DIR_NAME), data.join(APP_DIR_NAME)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_splits_into_config_and_data() {
        let p = resolve_with(Some(OsString::from(r"D:ampo-test"))).unwrap();
        assert_eq!(p.config_root, PathBuf::from(r"D:ampo-test").join("config"));
        assert_eq!(p.data_root, PathBuf::from(r"D:ampo-test").join("data"));
    }

    #[test]
    fn without_override_uses_appdata_and_localappdata() {
        for unset in [None, Some(OsString::new())] {
            let p = resolve_with(unset).unwrap();
            assert!(p.config_root.ends_with(APP_DIR_NAME));
            assert!(p.data_root.ends_with(APP_DIR_NAME));
            assert_ne!(p.config_root, p.data_root);
        }
    }
}
