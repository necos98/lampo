//! Local persistence and filesystem facts. Implemented by `lampo-infra-store`.
//! All synchronous: small files under `%APPDATA%\Lampo` / `%LOCALAPPDATA%\Lampo`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::detection::DetectionHint;
use crate::error::PortResult;
use crate::preset::{Preset, PresetId};
use crate::project::{Project, ProjectId, ProjectRoot};
use crate::project_import::ImportedProject;
use crate::scaffold::PathState;
use crate::settings::Settings;
use crate::shim::ShimIndex;
use crate::site::WorktreeSlug;
use crate::worktree::WorktreeEnvironment;

pub use crate::resolution::{GitInspector, PathCanonicalizer, ProjectResolver};

/// A project file that exists but does not load: malformed TOML, or values the domain rejects
/// (an empty address list, a database without MySQL, …). The project is hidden, not gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableProject {
    pub file: PathBuf,
    /// Why it does not load (names the file).
    pub reason: String,
    /// Loopback ports the file names, read leniently: they stay reserved, so a new site never
    /// takes the listener of a project the user is about to repair.
    pub loopback_ports: BTreeSet<u16>,
}

/// Every project file: the projects that load and the ones that do not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectListing {
    /// Sorted by id.
    pub projects: Vec<Project>,
    /// Sorted by file name.
    pub unreadable: Vec<UnreadableProject>,
}

/// Project configurations (one TOML file per project).
pub trait ProjectRepository: Send + Sync {
    /// Every project file, the unreadable ones reported instead of failing the listing.
    fn scan(&self) -> PortResult<ProjectListing>;
    /// The projects that load, sorted by id (see [`Self::scan`] for the others).
    fn list(&self) -> PortResult<Vec<Project>> {
        self.scan().map(|listing| listing.projects)
    }
    fn get(&self, id: &ProjectId) -> PortResult<Option<Project>>;
    /// Creates or replaces. Writes atomically (temp file + rename).
    fn save(&self, project: &Project) -> PortResult<()>;
    /// Removes the configuration only. Never touches the project folder.
    fn delete(&self, id: &ProjectId) -> PortResult<()>;
}

/// Global settings. Returns `Settings::default()` when the file does not exist yet.
pub trait SettingsRepository: Send + Sync {
    fn load(&self) -> PortResult<Settings>;
    fn save(&self, settings: &Settings) -> PortResult<()>;
}

/// User-defined presets (built-ins are not stored; see `preset::builtin`).
pub trait CustomPresetRepository: Send + Sync {
    fn list(&self) -> PortResult<Vec<Preset>>;
    fn save(&self, preset: &Preset) -> PortResult<()>;
    fn delete(&self, id: &PresetId) -> PortResult<()>;
}

/// Export / import of a single project's configuration as a portable file.
pub trait ProjectTransfer: Send + Sync {
    fn export_to(&self, project: &Project, file: &Path) -> PortResult<()>;
    /// Parses and validates an exported file, leaving out what an import never brings along
    /// (`project_import`: the Apache directives, the untrusted php.ini keys) and reporting it.
    /// The returned project is *not* saved.
    fn import_from(&self, file: &Path) -> PortResult<ImportedProject>;
}

/// Writes the CLI shim index (atomically; shims may be reading it concurrently).
pub trait ShimIndexWriter: Send + Sync {
    fn write(&self, index: &ShimIndex) -> PortResult<()>;
}

/// Reads the CLI shim index. Must be fast: called on every `php`/`composer` invocation.
/// A missing file yields an empty index.
pub trait ShimIndexReader: Send + Sync {
    fn read(&self) -> PortResult<ShimIndex>;
}

/// Places the shim executables into the shim dir: copies of `lampo-shim.exe` named after each
/// [`crate::shim::ShimTool`] (`php.exe`, `composer.exe`, `mysql.exe`, `mysqldump.exe`,
/// `redis-cli.exe`), plus a copy of the `lampo.exe` CLI so that one PATH entry provides it too.
/// Real `.exe` files, so they behave the same in PowerShell, cmd and Git Bash.
pub trait ShimInstaller: Send + Sync {
    fn install(&self, shim_dir: &Path) -> PortResult<()>;
    fn is_installed(&self, shim_dir: &Path) -> PortResult<bool>;
    /// After an app update the installed copies are the previous `lampo-shim.exe` / `lampo.exe`:
    /// brings every copy up to date with the current executables (byte comparison). Does nothing
    /// when nothing is installed at all (the user never installed the shims). Returns whether a
    /// file changed.
    fn refresh(&self, shim_dir: &Path) -> PortResult<bool>;
}

/// Records where the running app is installed, so that a copy of the CLI placed elsewhere (the
/// `lampo.exe` in the shim dir) still finds `lampo-desktop.exe` and the helper executables.
pub trait AppLocationRecorder: Send + Sync {
    /// Records the folder of the running app. Idempotent; the last started app wins.
    fn record_running_app(&self) -> PortResult<()>;
}

/// The current user's `PATH` (HKCU, no elevation needed).
pub trait UserPathEditor: Send + Sync {
    fn contains(&self, dir: &Path) -> PortResult<bool>;
    /// Appends `dir` if missing and broadcasts the environment change.
    fn append(&self, dir: &Path) -> PortResult<()>;
}

/// Recognises one kind of project from the files in its folder. One implementation per
/// framework; new detectors are added by implementing this trait (open-closed).
pub trait ProjectDetector: Send + Sync {
    /// Short, stable name for logs (`laravel`).
    fn name(&self) -> &str;
    /// `Ok(None)` when the folder is not this kind of project.
    fn detect(&self, folder: &Path) -> PortResult<Option<DetectionHint>>;
}

/// Minimal filesystem facts the use cases need to validate input and detect stale worktrees.
pub trait FileSystemInspector: Send + Sync {
    fn is_dir(&self, path: &Path) -> bool;
}

/// The files a new project's setup touches: its destination folder (checked before, removed
/// after a failure if this run created it) and Lampo's own tool files (`composer.bat`, the
/// Laravel Installer's folder).
pub trait ScaffoldFiles: Send + Sync {
    fn path_state(&self, path: &Path) -> PathState;
    fn is_file(&self, path: &Path) -> bool;
    fn create_dir_all(&self, path: &Path) -> PortResult<()>;
    /// Creates the parent folders, then writes `contents` (replacing the file).
    fn write_file(&self, path: &Path, contents: &str) -> PortResult<()>;
    /// Ok if absent. Clears read-only attributes (git object files are read-only).
    fn remove_dir_all(&self, path: &Path) -> PortResult<()>;
}

/// Worktree environments, stored next to their parent project
/// (`projects\<id>.worktrees\<slug>.toml`).
pub trait WorktreeRepository: Send + Sync {
    /// Every environment of every project.
    fn list_all(&self) -> PortResult<Vec<WorktreeEnvironment>>;
    fn list_for(&self, parent: &ProjectId) -> PortResult<Vec<WorktreeEnvironment>>;
    fn get(&self, parent: &ProjectId, slug: &WorktreeSlug) -> PortResult<Option<WorktreeEnvironment>>;
    /// The environment whose folder is `root` (case-insensitive), across all projects.
    fn find_by_root(&self, root: &ProjectRoot) -> PortResult<Option<WorktreeEnvironment>>;
    fn save(&self, environment: &WorktreeEnvironment) -> PortResult<()>;
    fn delete(&self, parent: &ProjectId, slug: &WorktreeSlug) -> PortResult<()>;
    /// Deletes all environments of a project (used when the project is deleted).
    fn delete_all_for(&self, parent: &ProjectId) -> PortResult<()>;
}

/// Looks up executables on the *effective* PATH (system + user, in Windows search order),
/// used by `doctor` to find a foreign `php.exe` shadowing the Lampo shims.
pub trait ExecutableLocator: Send + Sync {
    /// Every match of `name` (e.g. `php.exe`) in PATH order.
    fn find_all(&self, name: &str) -> PortResult<Vec<PathBuf>>;
}
