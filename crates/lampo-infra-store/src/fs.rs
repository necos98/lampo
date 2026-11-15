use std::path::Path;
use std::time::Duration;

use lampo_domain::ports::{AgentPluginFiles, FileSystemInspector, ScaffoldFiles};
use lampo_domain::scaffold::PathState;
use lampo_domain::{PortError, PortResult};

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalFileSystem;

impl FileSystemInspector for LocalFileSystem {
    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
}

/// Removal attempts: a just-killed process tree may hold files open for a moment.
const REMOVE_ATTEMPTS: u32 = 5;
const REMOVE_RETRY: Duration = Duration::from_millis(200);

/// Clears the read-only attribute of every file below `dir` (git makes its object files
/// read-only, which `remove_dir_all` cannot delete on Windows).
fn clear_read_only(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.is_dir() {
            clear_read_only(&path);
        } else if meta.permissions().readonly() {
            let mut perms = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(&path, perms);
        }
    }
}

impl ScaffoldFiles for LocalFileSystem {
    fn path_state(&self, path: &Path) -> PathState {
        match std::fs::metadata(path) {
            Err(_) => PathState::Missing,
            Ok(m) if m.is_dir() => match std::fs::read_dir(path).map(|mut entries| entries.next().is_none()) {
                Ok(true) => PathState::EmptyDir,
                _ => PathState::NonEmptyDir,
            },
            Ok(_) => PathState::File,
        }
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn create_dir_all(&self, path: &Path) -> PortResult<()> {
        std::fs::create_dir_all(path).map_err(|e| PortError::io(format!("creating {}", path.display()), e))
    }

    fn write_file(&self, path: &Path, contents: &str) -> PortResult<()> {
        if let Some(parent) = path.parent() {
            self.create_dir_all(parent)?;
        }
        std::fs::write(path, contents).map_err(|e| PortError::io(format!("writing {}", path.display()), e))
    }

    /// Blocks briefly while retrying (only used on the failure path of a scaffold).
    fn remove_dir_all(&self, path: &Path) -> PortResult<()> {
        let mut last = None;
        for attempt in 0..REMOVE_ATTEMPTS {
            if !path.exists() {
                return Ok(());
            }
            clear_read_only(path);
            match std::fs::remove_dir_all(path) {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => last = Some(e),
            }
            if attempt + 1 < REMOVE_ATTEMPTS {
                std::thread::sleep(REMOVE_RETRY);
            }
        }
        Err(PortError::io(format!("removing {}", path.display()), last.map(|e| e.to_string()).unwrap_or_default()))
    }
}

impl AgentPluginFiles for LocalFileSystem {
    fn read_text(&self, path: &Path) -> PortResult<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(PortError::io(format!("reading {}", path.display()), e)),
        }
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn list_dir(&self, dir: &Path) -> PortResult<Vec<String>> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(PortError::io(format!("listing {}", dir.display()), e)),
        };
        Ok(entries.flatten().filter_map(|e| e.file_name().into_string().ok()).collect())
    }

    fn write_text(&self, path: &Path, contents: &str) -> PortResult<()> {
        self.write_file(path, contents)
    }

    fn remove(&self, path: &Path) -> PortResult<()> {
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PortError::io(format!("removing {}", path.display()), e)),
            Ok(meta) if meta.is_dir() => self.remove_dir_all(path),
            Ok(_) => std::fs::remove_file(path).map_err(|e| PortError::io(format!("removing {}", path.display()), e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_states_writes_and_read_only_removal() {
        let tmp = tempfile::tempdir().unwrap();
        let fs = LocalFileSystem;
        let dir = tmp.path().join("shop");
        assert_eq!(fs.path_state(&dir), PathState::Missing);
        fs.create_dir_all(&dir).unwrap();
        assert_eq!(fs.path_state(&dir), PathState::EmptyDir);
        let file = dir.join(".git").join("objects").join("ab");
        fs.write_file(&file, "x").unwrap();
        assert!(fs.is_file(&file));
        assert_eq!(fs.path_state(&dir), PathState::NonEmptyDir);
        assert_eq!(fs.path_state(&file), PathState::File);
        let mut perms = std::fs::metadata(&file).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&file, perms).unwrap();
        fs.remove_dir_all(&dir).unwrap();
        assert_eq!(fs.path_state(&dir), PathState::Missing);
        fs.remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn agent_plugin_files_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let fs = LocalFileSystem;
        let root = tmp.path().join("claude-code");
        assert_eq!(fs.read_text(&root.join("x.json")).unwrap(), None);
        assert!(fs.list_dir(&root).unwrap().is_empty());
        fs.write_text(&root.join("skills").join("lampo").join("SKILL.md"), "---\n").unwrap();
        fs.write_text(&root.join("marker"), "1").unwrap();
        assert_eq!(std::fs::read(root.join("marker")).unwrap(), b"1", "no BOM");
        assert_eq!(fs.read_text(&root.join("marker")).unwrap().as_deref(), Some("1"));
        let mut names = fs.list_dir(&root).unwrap();
        names.sort();
        assert_eq!(names, ["marker", "skills"]);
        fs.remove(&root.join("marker")).unwrap();
        fs.remove(&root.join("skills")).unwrap();
        fs.remove(&root.join("missing")).unwrap();
        assert!(fs.list_dir(&root).unwrap().is_empty());
        assert!(fs.exists(&root) && !fs.exists(&root.join("skills")));
    }
}
