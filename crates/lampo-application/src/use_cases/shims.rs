use std::path::Path;
use std::sync::Arc;

use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{ExecutableLocator, ShimInstaller, UserPathEditor};
use lampo_domain::shim::ShimTool;

use crate::dto::{ShadowedToolDto, ShimStatusDto};
use crate::error::AppResult;

/// File extensions a shimmed tool may have elsewhere on PATH (`composer.bat` is common).
const EXECUTABLE_EXTENSIONS: &[&str] = &["exe", "bat", "cmd"];

fn same_dir(file: &Path, dir: &Path) -> bool {
    let norm = |p: &Path| p.display().to_string().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    file.parent().is_some_and(|parent| norm(parent) == norm(dir))
}

/// Tools whose first match on the effective PATH is not the Lampo shim. A failing lookup is
/// logged and treated as "no match".
pub(crate) fn shadowed_tools(locator: &dyn ExecutableLocator, shim_dir: &Path, tools: &[Arc<dyn ShimTool>]) -> Vec<ShadowedToolDto> {
    tools
        .iter()
        .filter_map(|tool| {
            let name = tool.name();
            let lookup = |ext: &str| match locator.find_all(&format!("{name}.{ext}")) {
                Ok(found) => found,
                Err(e) => {
                    tracing::warn!("PATH lookup of {name}.{ext} failed: {e}");
                    Vec::new()
                }
            };
            // `find_all` is per file name, so relative order across extensions is unknown: the
            // `.exe` order decides; other extensions only matter when no `.exe` is found at all.
            let exe = lookup("exe");
            let others: Vec<_> = EXECUTABLE_EXTENSIONS.iter().skip(1).flat_map(|ext| lookup(ext)).collect();
            let first = exe.first().or(others.first())?;
            (!same_dir(first, shim_dir)).then(|| ShadowedToolDto {
                tool: name.to_string(),
                shim: shim_dir.join(format!("{name}.exe")).display().to_string(),
                shadowed_by: first.display().to_string(),
            })
        })
        .collect()
}

/// Installs the `.exe` shims and puts their directory on the user PATH (no elevation).
pub struct ShimSetup {
    installer: Arc<dyn ShimInstaller>,
    user_path: Arc<dyn UserPathEditor>,
    locator: Arc<dyn ExecutableLocator>,
    paths: LampoPaths,
    /// The registered shim tools (the same list the installer copies).
    tools: Vec<Arc<dyn ShimTool>>,
}

impl ShimSetup {
    pub fn new(installer: Arc<dyn ShimInstaller>, user_path: Arc<dyn UserPathEditor>, locator: Arc<dyn ExecutableLocator>, paths: LampoPaths, tools: Vec<Arc<dyn ShimTool>>) -> Self {
        Self { installer, user_path, locator, paths, tools }
    }

    pub fn status(&self) -> AppResult<ShimStatusDto> {
        let dir = self.paths.shim_dir();
        Ok(ShimStatusDto {
            shim_dir: dir.display().to_string(),
            installed: self.installer.is_installed(&dir)?,
            on_path: self.user_path.contains(&dir)?,
            shadowed: shadowed_tools(self.locator.as_ref(), &dir, &self.tools),
        })
    }

    /// Brings already installed shims up to date with the current `lampo-shim.exe` (after an
    /// app update the copies are the old one). Never installs them nor touches the PATH.
    /// Returns whether a file changed.
    pub fn refresh(&self) -> AppResult<bool> {
        Ok(self.installer.refresh(&self.paths.shim_dir())?)
    }

    /// Idempotent: copies the shims (refreshing them after an app update) and appends the shim
    /// dir to the user PATH if missing.
    pub fn install(&self) -> AppResult<ShimStatusDto> {
        let dir = self.paths.shim_dir();
        self.installer.install(&dir)?;
        if !self.user_path.contains(&dir)? {
            self.user_path.append(&dir)?;
        }
        self.status()
    }

    /// Idempotent undo of [`Self::install`]: takes the shim dir off the user PATH first (so a
    /// copy still running, which cannot be deleted, never leaves a stale PATH entry behind),
    /// then deletes the copies (the shim index stays).
    pub fn uninstall(&self) -> AppResult<ShimStatusDto> {
        let dir = self.paths.shim_dir();
        self.user_path.remove(&dir)?;
        self.installer.uninstall(&dir)?;
        self.status()
    }
}
