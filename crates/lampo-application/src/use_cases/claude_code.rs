//! Claude Code integration (Settings > Integrazioni > Claude Code).
//!
//! Lampo generates a local plugin (`lampo_domain::claude_code`) into
//! `LampoPaths::claude_plugin_dir()` and registers it **only** through the official CLI, in user
//! scope; nothing is ever written inside a project. Install and removal happen only on an
//! explicit request; [`ClaudeCodeSetup::refresh_if_outdated`] (app startup) updates silently a
//! plugin that was installed before and is now outdated, and never installs one. An isolated
//! data root (`LAMPO_HOME`) never touches the real Claude Code.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use std::sync::Arc;

use lampo_domain::claude_code::{self, CliStep, INSTALLED_MARKER, PLUGIN_ID, PluginBundle, SkillFile};
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{AgentPluginFiles, ClaudeCodeCli, CommandOutput};

use crate::dto::{ClaudeCodeActionDto, ClaudeCodeStateDto, ClaudeCodeStatusDto, ClaudeCodeStepDto, StepOutcomeDto};
use crate::error::{AppError, AppResult, ErrorCode};

/// Upper bound of one `claude plugin …` command (an install may copy files into its cache).
const CLI_TIMEOUT: Duration = Duration::from_secs(120);
/// Output lines kept in a step's detail.
const DETAIL_LINES: usize = 4;

/// The skills shipped with this build: every file under `claude-plugin/skills/` in the
/// repository, embedded at compile time (`build.rs`).
pub fn embedded_skills() -> Vec<SkillFile> {
    const FILES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/claude_skills.rs"));
    FILES.iter().map(|(path, content)| SkillFile::new(*path, *content)).collect()
}

/// The installed plugin differs from the one this app would generate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutdatedPlugin {
    pub installed: String,
    pub expected: String,
}

#[derive(Default)]
struct StepLog(Vec<ClaudeCodeStepDto>);

impl StepLog {
    fn push(&mut self, step: &str, outcome: StepOutcomeDto, detail: Option<String>) {
        self.0.push(ClaudeCodeStepDto { step: step.to_string(), outcome, detail });
    }
    fn ok(&mut self, step: &str, detail: impl Into<Option<String>>) {
        self.push(step, StepOutcomeDto::Ok, detail.into());
    }
    /// Records the failure and returns `false` (for `return log.fail(...)`).
    fn fail(&mut self, step: &str, detail: impl Into<String>) -> bool {
        self.push(step, StepOutcomeDto::Failed, Some(detail.into()));
        false
    }
    /// `step: detail` of the last failed step.
    fn failure(&self) -> Option<String> {
        self.0.iter().rev().find(|s| s.outcome == StepOutcomeDto::Failed).map(|s| format!("{}: {}", s.step, s.detail.clone().unwrap_or_default()))
    }
}

/// The first lines of what a failing command printed (stderr first).
fn output_detail(out: &CommandOutput) -> String {
    let lines: Vec<&str> = out.stderr.lines().chain(out.stdout.lines()).map(str::trim).filter(|l| !l.is_empty()).take(DETAIL_LINES).collect();
    match (lines.is_empty(), out.exit_code) {
        (true, Some(code)) => format!("exit code {code}"),
        (true, None) => "no exit code".to_string(),
        (false, _) => lines.join("\n"),
    }
}

pub struct ClaudeCodeSetupDeps {
    pub cli: Arc<dyn ClaudeCodeCli>,
    pub files: Arc<dyn AgentPluginFiles>,
    pub paths: LampoPaths,
    /// Version of the running app (first part of the plugin version).
    pub app_version: String,
    /// Usually [`embedded_skills`].
    pub skills: Vec<SkillFile>,
    /// `LAMPO_HOME` is set: report [`ClaudeCodeStateDto::Isolated`] and do nothing.
    pub isolated: bool,
}

pub struct ClaudeCodeSetup {
    cli: Arc<dyn ClaudeCodeCli>,
    files: Arc<dyn AgentPluginFiles>,
    paths: LampoPaths,
    app_version: String,
    skills: Vec<SkillFile>,
    isolated: bool,
    /// Install, removal and refresh never overlap.
    busy: tokio::sync::Mutex<()>,
    update_error: Mutex<Option<String>>,
}

impl ClaudeCodeSetup {
    pub fn new(deps: ClaudeCodeSetupDeps) -> Self {
        let ClaudeCodeSetupDeps { cli, files, paths, app_version, skills, isolated } = deps;
        Self { cli, files, paths, app_version, skills, isolated, busy: tokio::sync::Mutex::new(()), update_error: Mutex::new(None) }
    }

    fn root(&self) -> PathBuf {
        self.paths.claude_plugin_dir()
    }

    fn bundle(&self) -> PluginBundle {
        claude_code::bundle(&self.app_version, &self.paths.shim_cli_exe().display().to_string(), &self.skills)
    }

    /// Version recorded by the last successful install; `None` if never installed (or removed).
    fn installed_version(&self) -> Option<String> {
        let text = self.files.read_text(&self.root().join(INSTALLED_MARKER)).ok().flatten()?;
        Some(text.trim().to_string()).filter(|v| !v.is_empty())
    }

    fn set_update_error(&self, error: Option<String>) {
        *self.update_error.lock().unwrap_or_else(|e| e.into_inner()) = error;
    }

    fn ensure_not_isolated(&self) -> AppResult<()> {
        if self.isolated {
            return Err(AppError::new(ErrorCode::Unsupported, "the Claude Code integration is disabled for an isolated data root (LAMPO_HOME)"));
        }
        Ok(())
    }

    async fn listed(&self) -> bool {
        match self.cli.run(&claude_code::list_args(), CLI_TIMEOUT).await {
            Ok(out) => claude_code::plugin_listed(&out.stdout, PLUGIN_ID),
            Err(e) => {
                tracing::warn!("claude plugin list failed: {e}");
                false
            }
        }
    }

    async fn detect(&self) -> Option<String> {
        match self.cli.detect().await {
            Ok(version) => version,
            Err(e) => {
                tracing::warn!("detecting Claude Code failed: {e}");
                None
            }
        }
    }

    fn status_of(&self, state: ClaudeCodeStateDto, claude_version: Option<String>) -> ClaudeCodeStatusDto {
        let cli = self.paths.shim_cli_exe();
        ClaudeCodeStatusDto {
            state,
            claude_version,
            installed_version: self.installed_version(),
            expected_version: self.bundle().version,
            plugin_dir: self.root().display().to_string(),
            cli_installed: self.files.exists(&cli),
            cli_path: cli.display().to_string(),
            update_error: self.update_error.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        }
    }

    /// Runs `claude --version` and `claude plugin list --json` (not under `LAMPO_HOME`).
    pub async fn status(&self) -> AppResult<ClaudeCodeStatusDto> {
        if self.isolated {
            return Ok(self.status_of(ClaudeCodeStateDto::Isolated, None));
        }
        let Some(version) = self.detect().await else {
            return Ok(self.status_of(ClaudeCodeStateDto::ClaudeMissing, None));
        };
        let state = match (self.listed().await, self.installed_version()) {
            (false, _) => ClaudeCodeStateDto::NotInstalled,
            (true, Some(installed)) if installed == self.bundle().version => ClaudeCodeStateDto::Installed,
            (true, _) => ClaudeCodeStateDto::Outdated,
        };
        Ok(self.status_of(state, Some(version)))
    }

    /// File reads only: the plugin was installed (marker present) with another version than the
    /// one this app generates. `None` when never installed, up to date, or isolated.
    pub fn find_outdated(&self) -> Option<OutdatedPlugin> {
        if self.isolated {
            return None;
        }
        let installed = self.installed_version()?;
        let expected = self.bundle().version;
        (installed != expected).then_some(OutdatedPlugin { installed, expected })
    }

    /// Writes the plugin and (re)registers it. Explicit user request only.
    pub async fn install(&self) -> AppResult<ClaudeCodeActionDto> {
        self.ensure_not_isolated()?;
        let busy = self.busy.lock().await;
        let mut log = StepLog::default();
        let ok = self.install_locked(&mut log).await;
        if ok {
            self.set_update_error(None);
        }
        drop(busy);
        Ok(ClaudeCodeActionDto { ok, steps: log.0, status: self.status().await? })
    }

    async fn install_locked(&self, log: &mut StepLog) -> bool {
        match self.detect().await {
            Some(version) => log.ok("detect-claude", version),
            None => return log.fail("detect-claude", "Claude Code (claude) not found"),
        }
        let cli = self.paths.shim_cli_exe();
        if !self.files.exists(&cli) {
            return log.fail("check-cli", cli.display().to_string());
        }
        log.ok("check-cli", cli.display().to_string());

        let root = self.root();
        let bundle = self.bundle();
        if let Err(e) = self.write_bundle(&bundle) {
            return log.fail("write-files", e.message);
        }
        log.ok("write-files", root.display().to_string());

        for step in claude_code::install_steps(&root.display().to_string()) {
            if !self.run_step(&step, log).await {
                return false;
            }
        }
        if !self.listed().await {
            return log.fail("verify", format!("{PLUGIN_ID} is not in `claude plugin list`"));
        }
        log.ok("verify", PLUGIN_ID.to_string());
        if let Err(e) = self.files.write_text(&root.join(INSTALLED_MARKER), &bundle.version) {
            return log.fail("write-marker", AppError::from(e).message);
        }
        log.ok("write-marker", bundle.version);
        true
    }

    /// Removes skill folders this build no longer ships, then writes every file.
    fn write_bundle(&self, bundle: &PluginBundle) -> AppResult<()> {
        let root = self.root();
        let skills_dir = root.join("plugins").join(claude_code::PLUGIN).join("skills");
        for name in self.files.list_dir(&skills_dir)? {
            if !bundle.skill_ids.contains(&name) {
                self.files.remove(&skills_dir.join(&name))?;
            }
        }
        for file in &bundle.files {
            let mut path = root.clone();
            path.extend(file.path.split('/'));
            self.files.write_text(&path, &file.content)?;
        }
        Ok(())
    }

    /// `false` when a non-lenient step failed.
    async fn run_step(&self, step: &CliStep, log: &mut StepLog) -> bool {
        let detail = match self.cli.run(&step.args, CLI_TIMEOUT).await {
            Ok(out) if out.success() => {
                log.ok(step.id, None);
                return true;
            }
            Ok(out) => output_detail(&out),
            Err(e) => AppError::from(e).message,
        };
        if step.lenient {
            log.push(step.id, StepOutcomeDto::Ignored, Some(detail));
            true
        } else {
            log.fail(step.id, detail)
        }
    }

    /// Unregisters the plugin and deletes the generated files. Explicit user request only.
    pub async fn uninstall(&self) -> AppResult<ClaudeCodeActionDto> {
        self.ensure_not_isolated()?;
        let busy = self.busy.lock().await;
        let mut log = StepLog::default();
        match self.detect().await {
            Some(version) => {
                log.ok("detect-claude", version);
                for step in claude_code::uninstall_steps() {
                    self.run_step(&step, &mut log).await;
                }
            }
            None => log.push("detect-claude", StepOutcomeDto::Ignored, Some("Claude Code (claude) not found: only the generated files are removed".into())),
        }
        let ok = match self.files.remove(&self.root()) {
            Ok(()) => {
                log.ok("remove-files", self.root().display().to_string());
                true
            }
            Err(e) => log.fail("remove-files", AppError::from(e).message),
        };
        self.set_update_error(None);
        drop(busy);
        Ok(ClaudeCodeActionDto { ok, steps: log.0, status: self.status().await? })
    }

    /// App startup: reinstalls a previously installed plugin whose version is not the expected
    /// one (an app update, or the CLI path changed). Never installs one that was not installed,
    /// and forgets one the user removed with `claude plugin uninstall`. A failure is logged and
    /// shown as a warning in the settings card. Returns whether the plugin was updated.
    pub async fn refresh_if_outdated(&self) -> bool {
        let Some(outdated) = self.find_outdated() else { return false };
        let _busy = self.busy.lock().await;
        if self.detect().await.is_none() {
            tracing::warn!("Claude Code plugin {} is outdated but `claude` was not found", outdated.installed);
            self.set_update_error(Some("detect-claude: Claude Code (claude) not found".into()));
            return false;
        }
        if !self.listed().await {
            tracing::info!("Claude Code plugin {PLUGIN_ID} was removed outside Lampo: not reinstalling it");
            if let Err(e) = self.files.remove(&self.root().join(INSTALLED_MARKER)) {
                tracing::warn!("removing the Claude Code plugin marker failed: {e}");
            }
            return false;
        }
        let mut log = StepLog::default();
        if self.install_locked(&mut log).await {
            tracing::info!("Claude Code plugin updated from {} to {}", outdated.installed, outdated.expected);
            self.set_update_error(None);
            true
        } else {
            let failure = log.failure().unwrap_or_else(|| "unknown failure".into());
            tracing::warn!("updating the Claude Code plugin failed: {failure}");
            self.set_update_error(Some(failure));
            false
        }
    }
}
