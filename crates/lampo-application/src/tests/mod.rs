//! Use-case tests against the in-memory fakes of [`crate::testing`].

mod allocation;
mod auto_restart;
mod claude_code;
mod doctor_defender;
mod doctor_services;
mod env_vars;
mod environment;
mod external_apps;
mod log_files;
mod misc;
mod mysql_accounts;
mod mysql_legacy_auth;
mod packages;
mod php_extension_names;
mod project_saves;
mod projects;
mod runtime_workers;
mod scaffold;
mod scan_exclusion;
mod service_lifecycle;
mod settings;
mod store_reads;
mod uninstall;
mod unreadable_config;
mod untrusted_config;
mod web_isolation;
mod which;
mod worktrees;

use crate::context::CallContext;

pub(crate) fn gui() -> CallContext {
    CallContext::GUI
}

/// The CLI default: never prompts for elevation.
pub(crate) fn cli() -> CallContext {
    CallContext::cli(false)
}
