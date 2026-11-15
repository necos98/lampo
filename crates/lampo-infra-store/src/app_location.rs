//! Where the app is installed, for CLI copies that do not sit next to it.
//!
//! The installer puts `lampo.exe` next to `lampo-desktop.exe`, and "Installa nel PATH" copies it
//! into the shim dir (`%LOCALAPPDATA%\Lampo\bin`). A copy there must still start the app headless
//! and use the helper executables (`lampo-shim.exe`, `lampo-hosts-helper.exe`), so the app writes
//! its own folder into `app-location.txt` (`LampoPaths::app_location_file`) at every start and the
//! CLI reads it when the app is not next to itself. Under `LAMPO_HOME` the file lives in that
//! isolated data root, so an isolated CLI copy finds the isolated app.

use std::path::{Path, PathBuf};

use lampo_domain::ports::AppLocationRecorder;
use lampo_domain::PortResult;

use crate::atomic::write_atomic;

/// File name of the desktop app, the marker of an app folder.
pub const APP_EXE_NAME: &str = "lampo-desktop.exe";

/// Writes `app_dir` (the running app's folder) into `file`.
pub struct AppLocationFile {
    file: PathBuf,
    app_dir: PathBuf,
}

impl AppLocationFile {
    pub fn new(file: PathBuf, app_dir: PathBuf) -> Self {
        Self { file, app_dir }
    }
}

impl AppLocationRecorder for AppLocationFile {
    /// Rewrites the file only when it names another folder.
    fn record_running_app(&self) -> PortResult<()> {
        let current = self.app_dir.display().to_string();
        if read_location(&self.file).is_some_and(|recorded| recorded == self.app_dir) {
            return Ok(());
        }
        write_atomic(&self.file, current.as_bytes())
    }
}

fn read_location(file: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(file).ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

fn is_app_dir(dir: &Path) -> bool {
    dir.join(APP_EXE_NAME).is_file()
}

/// The folder holding `lampo-desktop.exe` and the helper executables, for a CLI running from
/// `own_dir`: `own_dir` itself when the app is there (the install folder, `target\<profile>`),
/// else the folder recorded in `location_file` when it still holds the app, else `own_dir`
/// (errors then name the missing executable next to the CLI, as before).
pub fn resolve_app_dir(own_dir: &Path, location_file: &Path) -> PathBuf {
    if is_app_dir(own_dir) {
        return own_dir.to_path_buf();
    }
    match read_location(location_file) {
        Some(recorded) if is_app_dir(&recorded) => recorded,
        _ => own_dir.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_folder(root: &Path, name: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(APP_EXE_NAME), b"app").unwrap();
        dir
    }

    #[test]
    fn cli_next_to_the_app_uses_its_own_folder() {
        let root = tempfile::tempdir().unwrap();
        let install = app_folder(root.path(), "install");
        let other = app_folder(root.path(), "other");
        let file = root.path().join("app-location.txt");
        AppLocationFile::new(file.clone(), other).record_running_app().unwrap();
        assert_eq!(resolve_app_dir(&install, &file), install);
    }

    #[test]
    fn cli_copy_in_the_shim_dir_finds_the_recorded_app() {
        let root = tempfile::tempdir().unwrap();
        let install = app_folder(root.path(), "Programs\\Lampo");
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let file = root.path().join("app-location.txt");
        assert_eq!(resolve_app_dir(&bin, &file), bin, "nothing recorded yet: own folder");
        AppLocationFile::new(file.clone(), install.clone()).record_running_app().unwrap();
        assert_eq!(resolve_app_dir(&bin, &file), install);
    }

    #[test]
    fn a_recorded_folder_without_the_app_is_ignored() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        let file = root.path().join("app-location.txt");
        AppLocationFile::new(file.clone(), root.path().join("uninstalled")).record_running_app().unwrap();
        assert_eq!(resolve_app_dir(&bin, &file), bin);
    }

    #[test]
    fn recording_is_idempotent_and_the_last_started_app_wins() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("data").join("app-location.txt");
        let first = app_folder(root.path(), "first");
        let second = app_folder(root.path(), "second");
        AppLocationFile::new(file.clone(), first.clone()).record_running_app().unwrap();
        let written = std::fs::metadata(&file).unwrap().modified().unwrap();
        AppLocationFile::new(file.clone(), first.clone()).record_running_app().unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), written, "unchanged: not rewritten");
        assert_eq!(read_location(&file), Some(first));
        AppLocationFile::new(file.clone(), second.clone()).record_running_app().unwrap();
        assert_eq!(read_location(&file), Some(second));
    }
}
