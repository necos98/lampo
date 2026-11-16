//! Use-case tests against the in-memory fakes of [`crate::testing`].

mod allocation;
mod claude_code;
mod environment;
mod external_apps;
mod misc;
mod packages;
mod project_saves;
mod projects;
mod scaffold;
mod service_lifecycle;
mod settings;
mod store_reads;
mod uninstall;
mod untrusted_config;
mod web_isolation;
mod worktrees;

use crate::context::CallContext;

pub(crate) fn gui() -> CallContext {
    CallContext::GUI
}

/// The CLI default: never prompts for elevation.
pub(crate) fn cli() -> CallContext {
    CallContext::cli(false)
}
