//! Serde DTOs exchanged with front-ends: Tauri commands/events, `lampo --json` output and the
//! control pipe. **These are a public contract**:
//!
//! * field names are `camelCase` on the wire, enums are `kebab-case` strings;
//! * every type is mirrored by hand in `apps/desktop/src/lib/api/types.ts` — change both together;
//! * `lampo --json` output is documented as stable: add optional fields, never rename/remove
//!   (bump [`crate::dto::agent::AGENT_CONTEXT_SCHEMA_VERSION`] / the control protocol otherwise).
//!
//! DTOs carry plain strings/numbers (versions as `"8.3"`, ids as slugs, paths as Windows paths);
//! validation happens when use cases convert them into domain types.

pub mod agent;
pub mod app;
pub mod database;
pub mod diagnostics;
pub mod integrations;
pub mod logs;
pub mod package;
pub mod preset;
pub mod project;
pub mod scaffold;
pub mod service;
pub mod settings;
pub mod worktree;

pub use agent::*;
pub use app::*;
pub use database::*;
pub use diagnostics::*;
pub use integrations::*;
pub use logs::*;
pub use package::*;
pub use preset::*;
pub use project::*;
pub use scaffold::*;
pub use service::*;
pub use settings::*;
pub use worktree::*;
