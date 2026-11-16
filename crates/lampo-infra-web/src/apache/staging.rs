//! The generated config tree on disk: staging, promotion with rollback copy, and change
//! detection. Blocking file IO only (callers run it on the blocking pool); no rendering, no
//! process control.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lampo_domain::{PortError, PortResult};
use lampo_infra_fs::{ensure_dir, write_atomic};

use super::render::VHOSTS_DIR;

/// Name of the validation wrapper written inside the staging dir (never promoted).
pub const VALIDATION_WRAPPER: &str = "_lampo-validate.conf";

/// `<live>` (what Apache reads), `<live>.staging` (candidate) and `<live>.previous` (the tree
/// replaced by the last promotion, kept for manual rollback/diagnosis).
#[derive(Debug, Clone)]
pub struct ConfigTree {
    live: PathBuf,
}

impl ConfigTree {
    pub fn new(live: impl Into<PathBuf>) -> Self {
        Self { live: live.into() }
    }

    pub fn live_dir(&self) -> &Path {
        &self.live
    }

    pub fn staging_dir(&self) -> PathBuf {
        sibling(&self.live, ".staging")
    }

    pub fn previous_dir(&self) -> PathBuf {
        sibling(&self.live, ".previous")
    }

    /// `true` if the live tree holds exactly `files` (same set of vhost files, same bytes).
    pub fn live_matches(&self, files: &[(String, String)]) -> bool {
        let same_content = files.iter().all(|(rel, text)| std::fs::read(self.live.join(rel)).is_ok_and(|b| b == text.as_bytes()));
        same_content && list_vhost_files(&self.live).len() == files.iter().filter(|(rel, _)| rel.starts_with(VHOSTS_DIR)).count()
    }

    /// Recreates the staging dir with `files` (paths relative, `/`-separated). Returns its path.
    pub fn stage(&self, files: &[(String, String)]) -> PortResult<PathBuf> {
        let staging = self.staging_dir();
        remove_dir_if_exists(&staging)?;
        ensure_dir(&staging.join(VHOSTS_DIR))?;
        for (rel, text) in files {
            let path = staging.join(rel);
            if let Some(parent) = path.parent() {
                ensure_dir(parent)?;
            }
            std::fs::write(&path, text).map_err(|e| PortError::io(format!("writing {}", path.display()), e))?;
        }
        Ok(staging)
    }

    /// Writes an auxiliary file into the staging dir (validation wrapper).
    pub fn write_staged(&self, rel: &str, text: &str) -> PortResult<PathBuf> {
        let path = self.staging_dir().join(rel);
        std::fs::write(&path, text).map_err(|e| PortError::io(format!("writing {}", path.display()), e))?;
        Ok(path)
    }

    pub fn discard_staging(&self) -> PortResult<()> {
        remove_dir_if_exists(&self.staging_dir())
    }

    /// Makes the staged tree live. Fast path: two directory renames (`live` → `previous`,
    /// `staging` → `live`). If the live dir cannot be renamed (a file in it held open by an
    /// indexer or antivirus), falls back to replacing the files one by one, atomically each.
    pub fn promote(&self) -> PortResult<()> {
        let staging = self.staging_dir();
        let _ = std::fs::remove_file(staging.join(VALIDATION_WRAPPER));
        if let Some(parent) = self.live.parent() {
            ensure_dir(parent)?;
        }
        if !self.live.exists() {
            return rename_with_retry(&staging, &self.live);
        }
        let previous = self.previous_dir();
        remove_dir_if_exists(&previous)?;
        if rename_with_retry(&self.live, &previous).is_err() {
            return self.promote_by_copy();
        }
        if let Err(e) = rename_with_retry(&staging, &self.live) {
            // Put the old tree back so Apache keeps a configuration.
            let _ = rename_with_retry(&previous, &self.live);
            return Err(e);
        }
        Ok(())
    }

    fn promote_by_copy(&self) -> PortResult<()> {
        let staging = self.staging_dir();
        let staged = collect_files(&staging)?;
        for rel in &staged {
            let bytes = std::fs::read(staging.join(rel)).map_err(|e| PortError::io(format!("reading {}", rel.display()), e))?;
            write_atomic(&self.live.join(rel), &bytes)?;
        }
        for stale in list_vhost_files(&self.live) {
            if !staged.iter().any(|s| s == &Path::new(VHOSTS_DIR).join(&stale)) {
                let path = self.live.join(VHOSTS_DIR).join(&stale);
                std::fs::remove_file(&path).map_err(|e| PortError::io(format!("removing {}", path.display()), e))?;
            }
        }
        remove_dir_if_exists(&staging)
    }
}

fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = dir.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// File names (not paths) of `<dir>/vhosts/*.conf`.
fn list_vhost_files(dir: &Path) -> Vec<OsString> {
    std::fs::read_dir(dir.join(VHOSTS_DIR))
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.file_name()).filter(|n| Path::new(n).extension().is_some_and(|x| x == "conf")).collect())
        .unwrap_or_default()
}

/// Relative paths of the files in `root` (one level of sub-directories, which is all we write).
fn collect_files(root: &Path) -> PortResult<Vec<PathBuf>> {
    let mut out = Vec::new();
    let read = |dir: &Path| std::fs::read_dir(dir).map_err(|e| PortError::io(format!("listing {}", dir.display()), e));
    for entry in read(root)?.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            for sub in read(&path)?.filter_map(Result::ok) {
                if sub.path().is_file() {
                    out.push(Path::new(&entry.file_name()).join(sub.file_name()));
                }
            }
        } else {
            out.push(PathBuf::from(entry.file_name()));
        }
    }
    Ok(out)
}

fn remove_dir_if_exists(dir: &Path) -> PortResult<()> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PortError::io(format!("removing {}", dir.display()), e)),
    }
}

/// Windows refuses to rename a directory while something has a handle in it; such locks
/// (indexer, antivirus) are usually released within milliseconds.
fn rename_with_retry(from: &Path, to: &Path) -> PortResult<()> {
    let mut last = None;
    for attempt in 0..5u32 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(50 * u64::from(attempt + 1)));
    }
    Err(PortError::io(format!("renaming {} to {}", from.display(), to.display()), last.expect("at least one attempt")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(vhosts: &[&str]) -> Vec<(String, String)> {
        let mut out = vec![("httpd.conf".to_string(), "main".to_string())];
        out.extend(vhosts.iter().map(|v| (format!("vhosts/{v}.conf"), format!("vhost {v}"))));
        out
    }

    #[test]
    fn stage_then_promote_swaps_trees_and_keeps_previous() {
        let root = tempfile::tempdir().unwrap();
        let tree = ConfigTree::new(root.path().join("apache"));
        tree.stage(&files(&["shop", "blog"])).unwrap();
        tree.write_staged(VALIDATION_WRAPPER, "x").unwrap();
        tree.promote().unwrap();
        assert!(tree.live_matches(&files(&["shop", "blog"])));
        assert!(!tree.live_dir().join(VALIDATION_WRAPPER).exists());
        assert!(!tree.staging_dir().exists());

        tree.stage(&files(&["shop"])).unwrap();
        tree.promote().unwrap();
        assert!(tree.live_matches(&files(&["shop"])));
        assert!(!tree.live_matches(&files(&["shop", "blog"])));
        assert!(tree.previous_dir().join("vhosts").join("blog.conf").exists());
    }

    #[test]
    fn copy_fallback_removes_stale_vhosts() {
        let root = tempfile::tempdir().unwrap();
        let tree = ConfigTree::new(root.path().join("apache"));
        tree.stage(&files(&["shop", "blog"])).unwrap();
        tree.promote().unwrap();
        tree.stage(&files(&["shop@x"])).unwrap();
        tree.promote_by_copy().unwrap();
        assert!(tree.live_matches(&files(&["shop@x"])));
        assert!(!tree.staging_dir().exists());
    }

    #[test]
    fn discard_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let tree = ConfigTree::new(root.path().join("apache"));
        tree.discard_staging().unwrap();
        tree.stage(&files(&[])).unwrap();
        tree.discard_staging().unwrap();
        assert!(!tree.staging_dir().exists());
    }
}
