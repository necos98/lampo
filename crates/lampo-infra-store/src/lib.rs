//! # lampo-infra-store
//!
//! Local persistence and filesystem facts. One reason to change: *how Lampo stores and reads
//! its own small files* (TOML/JSON formats, file layout, Windows user environment).
//!
//! | Adapter                    | Ports                                   |
//! |----------------------------|-----------------------------------------|
//! | [`TomlProjectRepository`]  | `ProjectRepository`                     |
//! | [`TomlWorktreeRepository`] | `WorktreeRepository`                    |
//! | [`TomlSettingsRepository`] | `SettingsRepository`                    |
//! | [`TomlPresetRepository`]   | `CustomPresetRepository`                |
//! | [`TomlProjectTransfer`]    | `ProjectTransfer`                       |
//! | [`JsonShimIndex`]          | `ShimIndexReader`, `ShimIndexWriter`    |
//! | [`ExeShimInstaller`]       | `ShimInstaller`                         |
//! | [`AppLocationFile`]        | `AppLocationRecorder`                   |
//! | [`TomlProcessExclusionRecord`] | `ProcessExclusionRecordReader`, `ProcessExclusionRecordWriter` |
//! | [`detectors`]              | `ProjectDetector` (PHP manifest, Docker) |
//! | [`FsGitInspector`]         | `GitInspector`                          |
//! | [`FsPathCanonicalizer`]    | `PathCanonicalizer`                     |
//! | [`LocalFileSystem`]        | `FileSystemInspector`, `ScaffoldFiles`, `AgentPluginFiles` |
//! | [`WindowsUserPath`]        | `UserPathEditor`                        |
//! | [`ProcessOnlyUserPath`]    | `UserPathEditor` (isolated `LAMPO_HOME`: registry untouched) |
//! | [`PathExecutableLocator`]  | `ExecutableLocator`                     |
//! | [`SystemClock`]            | `Clock`                                 |
//!
//! Plus [`paths::resolve_lampo_paths`] and [`app_location::resolve_app_dir`] used by
//! composition roots.

pub mod app_location;
pub mod atomic;
pub mod clock;
pub mod defender_exclusion;
pub mod detectors;
pub mod final_path;
pub mod fs;
pub mod git;
pub mod locator;
pub mod path_list;
pub mod paths;
pub mod presets;
pub mod projects;
pub mod registry;
pub mod settings;
pub mod shim_index;
pub mod shim_installer;
pub mod transfer;
mod unreadable;
pub mod user_path;
pub mod worktrees;

#[cfg(test)]
mod testutil;

pub use app_location::AppLocationFile;
pub use clock::SystemClock;
pub use defender_exclusion::TomlProcessExclusionRecord;
pub use final_path::FsPathCanonicalizer;
pub use fs::LocalFileSystem;
pub use git::FsGitInspector;
pub use presets::TomlPresetRepository;
pub use projects::TomlProjectRepository;
pub use settings::TomlSettingsRepository;
pub use shim_index::JsonShimIndex;
pub use shim_installer::ExeShimInstaller;
pub use transfer::TomlProjectTransfer;
pub use user_path::{PathExecutableLocator, ProcessOnlyUserPath, WindowsUserPath};
pub use worktrees::TomlWorktreeRepository;
