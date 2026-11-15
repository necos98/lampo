//! Executable lookup on the *registry* PATH (machine then user), used by `doctor`.

use std::path::{Path, PathBuf};

use lampo_domain::PortResult;
use lampo_domain::ports::ExecutableLocator;

use crate::path_list;
use crate::registry::{self, Hive};
use crate::user_path::process_env;

const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Searches the *registry* PATH (machine then user), not the current process PATH, so the
/// answer matches what a fresh terminal will see.
#[derive(Debug, Default, Clone, Copy)]
pub struct PathExecutableLocator;

impl ExecutableLocator for PathExecutableLocator {
    fn find_all(&self, name: &str) -> PortResult<Vec<PathBuf>> {
        let mut dirs = Vec::new();
        for hive in [Hive::Machine, Hive::User] {
            if let Some(value) = registry::read_path(hive)? {
                dirs.extend(path_list::entries(&value.text).into_iter().map(|e| PathBuf::from(path_list::expand_vars(e.trim_matches('"'), &process_env))));
            }
        }
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| DEFAULT_PATHEXT.to_string());
        Ok(find_in_dirs(&dirs, name, &path_list::entries(&pathext), |p| p.is_file()))
    }
}

/// Every existing match of `name` in `dirs`, in directory order, then `pathext` order. `name`
/// that already ends in a PATHEXT extension is used as is; otherwise each extension is tried.
/// Duplicate directories (case-insensitive) yield one match.
pub fn find_in_dirs(dirs: &[PathBuf], name: &str, pathext: &[&str], is_file: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let lower = name.to_lowercase();
    let has_ext = pathext.iter().any(|e| lower.ends_with(&e.to_lowercase()));
    let candidates: Vec<String> = if has_ext { vec![name.to_string()] } else { pathext.iter().map(|e| format!("{name}{e}")).collect() };
    let mut seen = std::collections::HashSet::new();
    let mut found = Vec::new();
    for dir in dirs {
        for file in &candidates {
            let path = dir.join(file);
            if is_file(&path) && seen.insert(path.to_string_lossy().to_lowercase()) {
                found.push(path);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_in_path_order_with_pathext_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("php.exe"), "").unwrap();
        std::fs::write(a.join("php.bat"), "").unwrap();
        std::fs::write(b.join("php.exe"), "").unwrap();
        let ext = [".COM", ".EXE", ".BAT"];
        let dirs = vec![a.clone(), b.clone(), PathBuf::from(a.to_string_lossy().to_uppercase())];
        let found = find_in_dirs(&dirs, "php", &ext, |p| p.is_file());
        assert_eq!(found, vec![a.join("php.EXE"), a.join("php.BAT"), b.join("php.EXE")]);
        let exact = find_in_dirs(&dirs, "php.exe", &ext, |p| p.is_file());
        assert_eq!(exact, vec![a.join("php.exe"), b.join("php.exe")]);
        assert!(find_in_dirs(&dirs, "composer", &ext, |p| p.is_file()).is_empty());
    }
}
