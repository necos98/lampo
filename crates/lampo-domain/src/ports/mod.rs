//! Port traits: the only way the application layer reaches the outside world.
//!
//! Conventions (keep them when adding ports):
//! * **Small, role-specific traits** (interface segregation): a reader and a writer are separate
//!   traits even when one adapter implements both.
//! * **Object-safe** and `Send + Sync`: the application stores them as `Arc<dyn Port>`.
//! * **Sync vs async**: quick local operations on small files (repositories, the shim index,
//!   detectors, git metadata, registry lookups, port probes, log reads) are synchronous. Anything
//!   that can take more than a few milliseconds or waits on something external (network,
//!   archives, child processes, the control pipe, a UAC prompt) is `async` via `#[async_trait]`,
//!   and its future is `Send`. Adapters must not block the runtime inside async methods.
//! * **Errors**: every method returns [`crate::PortResult`]. Adapters map library errors into a
//!   [`crate::PortError`] category; they never panic on IO failures.
//! * **Live feedback**: long operations take a `&dyn ProgressSink` and a
//!   [`crate::progress::CancelFlag`]; asynchronous state changes (service crash, log lines) are
//!   pushed through an [`EventPublisher`] the adapter receives at construction.
//!
//! Which crate implements which port:
//!
//! | Module       | Implemented by          |
//! |--------------|-------------------------|
//! | [`store`]    | `lampo-infra-store` (also `GitInspector`, `PathCanonicalizer`, `ExecutableLocator`, `ScaffoldFiles`) |
//! | [`system`]   | `lampo-infra-store` (`SystemClock`), `lampo-infra-services` (`TokioTimer`) |
//! | [`packages`] | `lampo-infra-packages`  |
//! | [`web`]      | `lampo-infra-web` (also Apache's `ServiceRunner`) |
//! | [`services`] | `lampo-infra-services` (supervisor, runners, ports, logs, `DatabaseAdmin`) |
//! | [`control`]  | `lampo-infra-services` (named pipe, headless launcher) |
//! | [`agents`]   | `lampo-infra-services` (`ClaudeCodeCli`), `lampo-infra-store` (`AgentPluginFiles`) |
//! | [`events`]   | composition roots (Tauri app, CLI) |
//!
//! Strategy traits with pure implementations live next to their domain module and are
//! re-exported here: [`ProjectResolver`] (`resolution`), `PresetDefinition` (`preset`),
//! `ShimTool` (`shim`).

pub mod agents;
pub mod control;
pub mod events;
pub mod packages;
pub mod services;
pub mod store;
pub mod system;
pub mod web;

pub use system::{Clock, Timer};
pub use agents::{AgentPluginFiles, ClaudeCodeCli};
pub use control::{AppLauncher, ControlChannelClient, ControlChannelServer, ControlFrameSink, ControlRequestHandler};
pub use events::{EventPublisher, NullEventPublisher, NullProgressSink, ProgressSink};
pub use packages::{
    ArchiveExtractor, InstalledPackageRegistry, PackageCatalogSource, PackageDownloader, PackageInstallHook, PackageLeftoverSweeper, PackageSlotLock, PhpExtensionScanner, SlotGuard,
    SlotReplacement, VcRuntimeDetector,
};
pub use services::{
    ChildId, CommandOutput, CommandRunner, CommandSpec, DatabaseAdmin, LogFileReader, LogRotator, MysqlEndpoint, PipedCommand, PipedCommandRunner, PortAvailability, PortProbe,
    ProcessObserver, ProcessSupervisor, ServiceRunner, SpawnedChild, StreamCompression, StreamingCommandRunner,
};
pub use store::{
    AppLocationRecorder, CustomPresetRepository, ExecutableLocator, FileSystemInspector, GitInspector, PathCanonicalizer, ProjectDetector, ProjectListing, ProjectRepository,
    ProjectResolver, ProjectTransfer, ScaffoldFiles, SettingsRepository, ShimIndexReader, ShimIndexWriter, ShimInstaller, UnreadableProject, UserPathEditor, WorktreeRepository,
};
pub use web::{HostsFileReader, HostsFileWriter, PhpIniWriter, WebServerConfigurator};
