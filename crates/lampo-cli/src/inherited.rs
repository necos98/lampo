//! What a parent `lampo exec`, `lampo env` or the app's terminal left in the environment for
//! site resolution, read the same way by `lampo` and by the shims.

use lampo_domain::resolution::env_vars;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InheritedEnv {
    /// `LAMPO_PROJECT`, if set and non-empty.
    pub site: Option<String>,
    /// `LAMPO_PROJECT_ROOT`, if set and non-empty.
    pub root: Option<String>,
    /// `LAMPO_QUIET`: no notes on stderr.
    pub quiet: bool,
}

impl InheritedEnv {
    pub fn from_process() -> Self {
        Self { site: var(env_vars::PROJECT), root: var(env_vars::PROJECT_ROOT), quiet: env_vars::is_quiet(std::env::var(env_vars::QUIET).ok().as_deref()) }
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}
