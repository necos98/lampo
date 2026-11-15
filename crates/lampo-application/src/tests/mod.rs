//! Use-case tests against the in-memory fakes of [`crate::testing`].

mod allocation;
mod claude_code;
mod environment;
mod misc;
mod packages;
mod projects;
mod scaffold;
mod settings;
mod untrusted_config;
mod worktrees;

use crate::context::CallContext;

pub(crate) fn gui() -> CallContext {
    CallContext::GUI
}

/// The CLI default: never prompts for elevation.
pub(crate) fn cli() -> CallContext {
    CallContext::cli(false)
}
