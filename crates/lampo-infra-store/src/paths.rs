//! Resolves the two Lampo roots from the Windows environment.
//!
//! The data root comes from `%LOCALAPPDATA%` as the process environment has it, and the Windows
//! known folder only when the variable is missing: the shims run on every `php` call and the
//! known-folder lookup (`SHGetKnownFolderPath`) costs milliseconds. The app, `lampo` and the
//! shims share this logic, so they agree on where the shim index lives.

use std::ffi::OsString;
use std::path::PathBuf;

use lampo_domain::paths::LampoPaths;
use lampo_domain::{PortError, PortResult};

/// Folder name under `%APPDATA%` and `%LOCALAPPDATA%`.
pub const APP_DIR_NAME: &str = "Lampo";

/// Environment variable overriding both roots (`<dir>\config`, `<dir>\data`), for tests and
/// portable setups.
pub const HOME_OVERRIDE_VAR: &str = "LAMPO_HOME";

/// Environment variable of the machine-local application data folder.
pub const LOCAL_APP_DATA_VAR: &str = "LOCALAPPDATA";

/// Sub-folders of a [`HOME_OVERRIDE_VAR`] folder.
const HOME_CONFIG_DIR: &str = "config";
const HOME_DATA_DIR: &str = "data";

/// `%APPDATA%\Lampo` and `%LOCALAPPDATA%\Lampo` (or `LAMPO_HOME`). Does not create them.
pub fn resolve_lampo_paths() -> PortResult<LampoPaths> {
    resolve_with(std::env::var_os(HOME_OVERRIDE_VAR))
}

/// `%LOCALAPPDATA%\Lampo` (or `LAMPO_HOME\data`) alone, for the shims: the config root is not
/// computed. Does not create it.
pub fn resolve_data_root() -> PortResult<PathBuf> {
    data_root_with(std::env::var_os(HOME_OVERRIDE_VAR), std::env::var_os(LOCAL_APP_DATA_VAR))
}

/// The `LAMPO_HOME` override, when set (non-empty). An isolated instance must not share
/// machine-wide state with the real one: composition roots use it to pick a separate control
/// pipe and to leave the user PATH alone.
pub fn home_override() -> Option<PathBuf> {
    non_empty(std::env::var_os(HOME_OVERRIDE_VAR))
}

/// [`resolve_lampo_paths`] with the override passed in (pure, testable without touching the
/// process environment). An empty override counts as unset.
pub fn resolve_with(home_override: Option<OsString>) -> PortResult<LampoPaths> {
    if let Some(home) = non_empty(home_override) {
        return Ok(LampoPaths::new(home.join(HOME_CONFIG_DIR), home.join(HOME_DATA_DIR)));
    }
    let config = dirs::config_dir().ok_or_else(|| PortError::io("locating %APPDATA%", "not available"))?;
    let data = data_root_with(None, std::env::var_os(LOCAL_APP_DATA_VAR))?;
    Ok(LampoPaths::new(config.join(APP_DIR_NAME), data))
}

/// [`resolve_data_root`] with the variables passed in. A relative `%LOCALAPPDATA%` is not
/// trusted (it would follow the current folder) and counts as unset.
pub fn data_root_with(home_override: Option<OsString>, local_app_data: Option<OsString>) -> PortResult<PathBuf> {
    if let Some(home) = non_empty(home_override) {
        return Ok(home.join(HOME_DATA_DIR));
    }
    let local = non_empty(local_app_data).filter(|p| p.is_absolute()).or_else(dirs::data_local_dir);
    let local = local.ok_or_else(|| PortError::io("locating %LOCALAPPDATA%", "not available"))?;
    Ok(local.join(APP_DIR_NAME))
}

fn non_empty(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_splits_into_config_and_data() {
        let p = resolve_with(Some(OsString::from(r"D:\lampo-test"))).unwrap();
        assert_eq!(p.config_root, PathBuf::from(r"D:\lampo-test").join("config"));
        assert_eq!(p.data_root, PathBuf::from(r"D:\lampo-test").join("data"));
        assert_eq!(data_root_with(Some(OsString::from(r"D:\lampo-test")), Some(OsString::from(r"C:\ignored"))).unwrap(), p.data_root);
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

    #[test]
    fn data_root_comes_from_the_environment_variable() {
        let root = data_root_with(None, Some(OsString::from(r"E:\Users\me\AppData\Local"))).unwrap();
        assert_eq!(root, PathBuf::from(r"E:\Users\me\AppData\Local").join(APP_DIR_NAME));
    }

    #[test]
    fn missing_or_relative_variable_falls_back_to_the_known_folder() {
        let known = dirs::data_local_dir().unwrap().join(APP_DIR_NAME);
        for fallback in [None, Some(OsString::new()), Some(OsString::from(r"relative\dir"))] {
            assert_eq!(data_root_with(None, fallback.clone()).unwrap(), known, "{fallback:?}");
        }
    }
}
