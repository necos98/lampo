//! Use cases. Each struct is one cohesive capability, built from `Arc<dyn Port>`s (and, where a
//! use case composes another, from the application-level traits [`EnvironmentSync`] and
//! [`ServiceEnsurer`]). Front-ends (Tauri commands, CLI, control-pipe dispatcher) only call these
//! and map DTOs; they contain no business logic.
//!
//! Which front-end may call what:
//! * read-only, file-based use cases (queries, resolution, agent context, env, logs, doctor
//!   without live service state) run anywhere, including the CLI when the app is not running;
//! * anything that starts/stops processes or mutates config runs **inside the app** (the single
//!   owner of processes). The CLI reaches them through the control pipe (`crate::control`).

pub mod agent;
pub mod allocation;
pub mod app_info;
pub mod claude_code;
pub mod database;
pub mod diagnostics;
pub mod environment;
pub(crate) mod generation;
pub mod lifecycle;
pub mod logs;
pub mod packages;
pub mod presets;
pub mod project_patch;
pub mod project_transfer;
pub mod projects;
pub(crate) mod requirements;
pub mod resolution;
pub mod scaffold;
pub mod service_catalog;
pub mod services;
pub mod settings;
pub mod settings_patch;
pub mod shims;
pub mod status;
pub(crate) mod support;
pub mod worktrees;

pub use agent::{AgentContextDeps, AgentContextQuery, ToolchainQuery};
pub use allocation::{LoopbackPortAllocator, SiteSnapshot};
pub use app_info::AppInfoQuery;
pub use claude_code::{ClaudeCodeSetup, ClaudeCodeSetupDeps, embedded_icon, embedded_skills};
pub use database::DatabaseService;
pub use diagnostics::{
    AppCheck, Doctor, DoctorCheck, DoctorDeps, DoctorFacts, FoldersCheck, HostsBlockCheck, PackagesCheck, PathShadowingCheck, PortsCheck, ShimsOnPathCheck, StaleWorktreesCheck,
    VcRuntimeCheck,
};
pub use environment::{ApplyEnvironment, ApplyEnvironmentDeps, EnvironmentSync, GeneratedFilesWriter};
pub use lifecycle::{FirstRunDeps, FirstRunSetup, Shutdown, ShutdownGate, Startup};
pub use logs::LogQueries;
pub use packages::{InstallPackage, InstallPackageDeps, PackageCatalog, PackageCatalogDeps, PhpExtensionListing, PhpExtensionsQuery, UninstallPackage, UninstallPackageDeps};
pub use presets::{DeletePreset, PresetQueries, SavePreset};
pub use project_patch::PatchProjectEnvironment;
pub use project_transfer::{ExportProject, ImportProject};
pub use projects::{CreateProject, DeleteProject, DetectProject, ProjectQueries, ProjectWriter, RelinkProject, SetProjectActive, SetProjectPhp, SuggestAddresses, UpdateProject};
pub use resolution::SiteResolver;
pub use scaffold::{ScaffoldProject, ScaffoldProjectDeps};
pub use service_catalog::ServiceCatalog;
pub use services::{OfflineServiceStatus, ServiceControl, ServiceControlDeps, ServiceEnsurer, ServiceQueries, ServiceStatusSource};
pub use settings::{SettingsQuery, SettingsService};
pub use settings_patch::{PatchSettings, PatchSettingsDeps};
pub use shims::ShimSetup;
pub use status::StatusQuery;
pub use worktrees::{PruneWorktrees, WorktreeDatabases, WorktreeDown, WorktreeQueries, WorktreeUp, WorktreeUpDeps};
