//! What every command handler receives: ports (role traits) plus the process facts that handlers
//! must not read themselves (cwd, `LAMPO_PROJECT`, clock), so tests are deterministic.

use std::sync::Arc;
use std::time::Duration;

use lampo_application::dto::ResolveRequestDto;

use crate::cli::GlobalArgs;
use crate::inherited::InheritedEnv;
use crate::ports::*;

/// Process facts captured once at startup.
#[derive(Debug, Clone)]
pub struct CliEnv {
    pub cwd: String,
    /// `LAMPO_PROJECT`, `LAMPO_PROJECT_ROOT`, `LAMPO_QUIET`.
    pub inherited: InheritedEnv,
    pub now_unix_ms: u64,
    /// Poll interval of `logs --follow`.
    pub follow_interval: Duration,
}

impl CliEnv {
    pub fn from_process() -> Self {
        Self {
            cwd: std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(),
            inherited: InheritedEnv::from_process(),
            now_unix_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            follow_interval: Duration::from_millis(500),
        }
    }

    /// Resolution inputs: `--project` always wins; `LAMPO_PROJECT` wins over the cwd only inside
    /// `LAMPO_PROJECT_ROOT` or outside every project.
    pub fn resolve_request(&self, global: &GlobalArgs) -> ResolveRequestDto {
        ResolveRequestDto { cwd: self.cwd.clone(), explicit_site: global.project.clone(), inherited_site: self.inherited.site.clone(), inherited_root: self.inherited.root.clone() }
    }

    /// Makes `path` absolute against the cwd (the app runs elsewhere) and folds `.` / `..`
    /// lexically (project roots reject them). No symlink resolution.
    pub fn absolute(&self, path: &str) -> String {
        use std::path::{Component, Path, PathBuf};
        let p = Path::new(path);
        let joined = if p.is_absolute() { p.to_path_buf() } else { Path::new(&self.cwd).join(p) };
        let mut out = PathBuf::new();
        for component in joined.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    // Never climb above the drive/share root.
                    if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                        out.pop();
                    }
                }
                other => out.push(other.as_os_str()),
            }
        }
        out.display().to_string()
    }
}

/// The ports used by the commands. Built by `composition::build`; faked in tests.
pub struct CliContext {
    pub env: CliEnv,
    pub sites: Arc<dyn SiteLookup>,
    pub projects: Arc<dyn ProjectReader>,
    pub detection: Arc<dyn ProjectDetection>,
    pub worktrees: Arc<dyn WorktreeReader>,
    pub logs: Arc<dyn LogReader>,
    pub log_opener: Arc<dyn LogOpener>,
    pub packages: Arc<dyn PackageReader>,
    pub php_extensions: Arc<dyn PhpExtensionReader>,
    pub toolchain: Arc<dyn ToolchainReader>,
    pub shim_tools: Arc<dyn ShimToolReader>,
    pub opener: Arc<dyn SiteOpener>,
    pub doctor: Arc<dyn DoctorRunner>,
    pub agent: Arc<dyn AgentContextReader>,
    pub hook_input: Arc<dyn HookInput>,
    pub shims: Arc<dyn ShimAdmin>,
    pub offline_status: Arc<dyn OfflineStatus>,
    pub app: Arc<dyn AppGateway>,
    pub runner: Arc<dyn ProgramRunner>,
    pub interrupt: Arc<dyn Interrupt>,
    pub settings: Arc<dyn SettingsReader>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_paths_are_lexically_normalised() {
        let env = CliEnv { cwd: r"C:\work\shop".into(), inherited: InheritedEnv::default(), now_unix_ms: 0, follow_interval: Duration::ZERO };
        assert_eq!(env.absolute("."), r"C:\work\shop");
        assert_eq!(env.absolute(r".\wt\..\api"), r"C:\work\shop\api");
        assert_eq!(env.absolute(r"..\..\..\x"), r"C:\x");
        assert_eq!(env.absolute(r"D:\a\.\b"), r"D:\a\b");
    }

    #[test]
    fn project_flag_and_inherited_variables_stay_apart() {
        let inherited = InheritedEnv { site: Some("shop".into()), root: Some(r"C:\work\shop".into()), quiet: false };
        let env = CliEnv { cwd: r"C:\work\blog".into(), inherited, now_unix_ms: 0, follow_interval: Duration::ZERO };
        let global = |project: Option<&str>| GlobalArgs { json: false, project: project.map(Into::into), allow_elevation: false, no_start: false };
        let request = env.resolve_request(&global(None));
        assert_eq!(request.explicit_site, None);
        assert_eq!((request.inherited_site.as_deref(), request.inherited_root.as_deref()), (Some("shop"), Some(r"C:\work\shop")));
        assert_eq!(env.resolve_request(&global(Some("api"))).explicit_site.as_deref(), Some("api"));
    }
}
