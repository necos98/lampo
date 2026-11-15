//! The Windows user environment: `HKCU\Environment\Path` (editing). The PATH-string logic is in
//! [`crate::path_list`]; the registry calls are in [`crate::registry`]. The executable locator
//! lives in [`crate::locator`] and is re-exported here.

use std::path::Path;

use lampo_domain::PortResult;
use lampo_domain::ports::UserPathEditor;

pub use crate::locator::PathExecutableLocator;
use crate::path_list;
use crate::registry::{self, Hive, PathValue};

pub(crate) fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Edits `HKCU\Environment\Path` (REG_EXPAND_SZ preserved) and broadcasts `WM_SETTINGCHANGE`.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsUserPath;

impl UserPathEditor for WindowsUserPath {
    fn contains(&self, dir: &Path) -> PortResult<bool> {
        let current = registry::read_path(Hive::User)?;
        Ok(current.is_some_and(|v| path_list::contains(&v.text, dir, &process_env)))
    }

    fn append(&self, dir: &Path) -> PortResult<()> {
        let current = registry::read_path(Hive::User)?;
        let updated = match &current {
            Some(v) => {
                if path_list::contains(&v.text, dir, &process_env) {
                    return Ok(());
                }
                PathValue { text: path_list::append(&v.text, dir, &process_env), expandable: v.expandable }
            }
            None => PathValue { text: dir.to_string_lossy().into_owned(), expandable: true },
        };
        registry::write_user_path(&updated)?;
        registry::broadcast_environment_change();
        Ok(())
    }
}

/// For isolated data roots (`LAMPO_HOME`: tests, portable setups): never touches the registry.
/// `contains` looks at the current process `PATH` (what the caller's shell put there), `append`
/// only logs. Composition roots choose it instead of [`WindowsUserPath`].
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessOnlyUserPath;

impl UserPathEditor for ProcessOnlyUserPath {
    fn contains(&self, dir: &Path) -> PortResult<bool> {
        Ok(process_env("PATH").is_some_and(|list| path_list::contains(&list, dir, &process_env)))
    }

    fn append(&self, dir: &Path) -> PortResult<()> {
        tracing::info!("isolated data root: the user PATH is left unchanged (add {} to PATH in your own shell)", dir.display());
        Ok(())
    }
}
