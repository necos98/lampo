//! Claude Code integration: install, removal, startup refresh, isolation.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::claude_code::SkillFile;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{AgentPluginFiles, ClaudeCodeCli, CommandOutput};
use lampo_domain::{PortError, PortResult};

use crate::dto::{ClaudeCodeStateDto, StepOutcomeDto};
use crate::error::ErrorCode;
use crate::use_cases::{ClaudeCodeSetup, ClaudeCodeSetupDeps, embedded_skills};

const ROOT: &str = r"C:\data\agent-plugins\claude-code";
const MARKER: &str = r"C:\data\agent-plugins\claude-code\.installed-version";
const CLI: &str = r"C:\data\bin\lampo.exe";
const LISTED: &str = r#"[{"id":"lampo@lampo","scope":"user"}]"#;

fn key(p: &Path) -> String {
    p.display().to_string().replace('/', "\\").to_lowercase()
}

/// `claude` with a canned version, `plugin list` output and failing subcommands.
struct FakeClaude {
    version: Option<String>,
    list: Mutex<String>,
    /// Args (joined with spaces) starting with one of these exit 1.
    failing: Mutex<Vec<String>>,
    calls: Mutex<Vec<String>>,
}

impl FakeClaude {
    fn new(version: Option<&str>, list: &str) -> Arc<Self> {
        Arc::new(Self { version: version.map(str::to_string), list: Mutex::new(list.into()), failing: Mutex::default(), calls: Mutex::default() })
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn fail(&self, prefix: &str) {
        self.failing.lock().unwrap().push(prefix.into());
    }
}

#[async_trait]
impl ClaudeCodeCli for FakeClaude {
    async fn detect(&self) -> PortResult<Option<String>> {
        self.calls.lock().unwrap().push("--version".into());
        Ok(self.version.clone())
    }
    async fn run(&self, args: &[String], _timeout: Duration) -> PortResult<CommandOutput> {
        let line = args.join(" ");
        self.calls.lock().unwrap().push(line.clone());
        if self.version.is_none() {
            return Err(PortError::not_found("program", "claude"));
        }
        if self.failing.lock().unwrap().iter().any(|f| line.starts_with(f.as_str())) {
            return Ok(CommandOutput { exit_code: Some(1), stdout: String::new(), stderr: format!("boom: {line}\n") });
        }
        let stdout = if line == "plugin list --json" { self.list.lock().unwrap().clone() } else { "ok".into() };
        if line.starts_with("plugin install") {
            *self.list.lock().unwrap() = LISTED.into();
        }
        if line.starts_with("plugin uninstall") {
            *self.list.lock().unwrap() = "[]".into();
        }
        Ok(CommandOutput { exit_code: Some(0), stdout, stderr: String::new() })
    }
}

/// In-memory text files (key: lowercase backslash path) plus existing non-text files.
#[derive(Default)]
struct MemFiles {
    files: Mutex<BTreeMap<String, String>>,
}

impl MemFiles {
    fn put(&self, path: &str, content: &str) {
        self.files.lock().unwrap().insert(key(Path::new(path)), content.into());
    }
    fn get(&self, path: &str) -> Option<String> {
        self.files.lock().unwrap().get(&key(Path::new(path))).cloned()
    }
    fn under(&self, dir: &str) -> Vec<String> {
        let prefix = format!("{}\\", key(Path::new(dir)));
        self.files.lock().unwrap().keys().filter(|k| k.starts_with(&prefix)).cloned().collect()
    }
}

impl AgentPluginFiles for MemFiles {
    fn read_text(&self, path: &Path) -> PortResult<Option<String>> {
        Ok(self.files.lock().unwrap().get(&key(path)).cloned())
    }
    fn exists(&self, path: &Path) -> bool {
        let k = key(path);
        self.files.lock().unwrap().keys().any(|f| *f == k || f.starts_with(&format!("{k}\\")))
    }
    fn list_dir(&self, dir: &Path) -> PortResult<Vec<String>> {
        let prefix = format!("{}\\", key(dir));
        let mut names: Vec<String> = self.files.lock().unwrap().keys().filter_map(|k| k.strip_prefix(&prefix).map(|rest| rest.split('\\').next().unwrap_or_default().to_string())).collect();
        names.dedup();
        Ok(names)
    }
    fn write_text(&self, path: &Path, contents: &str) -> PortResult<()> {
        self.files.lock().unwrap().insert(key(path), contents.into());
        Ok(())
    }
    fn remove(&self, path: &Path) -> PortResult<()> {
        let k = key(path);
        self.files.lock().unwrap().retain(|f, _| *f != k && !f.starts_with(&format!("{k}\\")));
        Ok(())
    }
}

fn skills() -> Vec<SkillFile> {
    vec![SkillFile::new("lampo/SKILL.md", "---\nname: lampo\ndescription: Core.\n---\n"), SkillFile::new("lampo-db/SKILL.md", "---\nname: lampo-db\ndescription: DB.\n---\n")]
}

fn setup_with(claude: Arc<FakeClaude>, files: Arc<MemFiles>, app_version: &str, isolated: bool) -> ClaudeCodeSetup {
    ClaudeCodeSetup::new(ClaudeCodeSetupDeps { cli: claude, files, paths: LampoPaths::new(r"C:\cfg", r"C:\data"), app_version: app_version.into(), skills: skills(), isolated })
}

fn world(version: Option<&str>, list: &str) -> (Arc<FakeClaude>, Arc<MemFiles>, ClaudeCodeSetup) {
    let claude = FakeClaude::new(version, list);
    let files = Arc::new(MemFiles::default());
    files.put(CLI, "MZ");
    let setup = setup_with(claude.clone(), files.clone(), "0.3.0", false);
    (claude, files, setup)
}

fn steps(action: &crate::dto::ClaudeCodeActionDto) -> Vec<(String, StepOutcomeDto)> {
    action.steps.iter().map(|s| (s.step.clone(), s.outcome)).collect()
}

#[tokio::test]
async fn install_writes_the_plugin_registers_it_in_user_scope_and_marks_the_version() {
    let (claude, files, setup) = world(Some("2.1.0 (Claude Code)"), "[]");
    files.put(&format!(r"{ROOT}\plugins\lampo\skills\obsolete\SKILL.md"), "old");
    let action = setup.install().await.unwrap();
    assert!(action.ok, "{:?}", action.steps);
    assert_eq!(action.status.state, ClaudeCodeStateDto::Installed);
    let expected = action.status.expected_version.clone();
    assert!(expected.starts_with("0.3.0-h"));
    assert_eq!(files.get(MARKER).as_deref(), Some(expected.as_str()));
    assert!(files.get(&format!(r"{ROOT}\.claude-plugin\marketplace.json")).is_some());
    let hooks = files.get(&format!(r"{ROOT}\plugins\lampo\hooks\hooks.json")).unwrap();
    assert!(hooks.contains("C:/data/bin/lampo.exe"), "{hooks}");
    assert!(files.get(&format!(r"{ROOT}\plugins\lampo\skills\lampo-db\SKILL.md")).is_some());
    assert!(files.under(&format!(r"{ROOT}\plugins\lampo\skills\obsolete")).is_empty(), "obsolete skill removed");

    let calls = claude.calls();
    let market = "C:/data/agent-plugins/claude-code";
    for expected in [
        format!("plugin validate {market}/plugins/lampo"),
        format!("plugin validate {market}"),
        "plugin uninstall lampo@lampo --scope user".to_string(),
        "plugin marketplace remove lampo".to_string(),
        format!("plugin marketplace add {market}"),
        "plugin install lampo@lampo --scope user".to_string(),
        "plugin list --json".to_string(),
    ] {
        assert!(calls.contains(&expected), "{expected} in {calls:?}");
    }
    assert!(calls.iter().all(|c| !c.contains("--scope project") && !c.contains("--scope local")));
    let ids: Vec<String> = steps(&action).into_iter().map(|(s, _)| s).collect();
    assert_eq!(ids[..4], ["detect-claude", "check-cli", "write-files", "validate-plugin"]);
    assert_eq!(ids.last().map(String::as_str), Some("write-marker"));
}

#[tokio::test]
async fn failing_validation_stops_before_registering_and_writes_no_marker() {
    let (claude, files, setup) = world(Some("2.1.0"), "[]");
    claude.fail("plugin validate");
    let action = setup.install().await.unwrap();
    assert!(!action.ok);
    assert_eq!(steps(&action).last().unwrap(), &("validate-plugin".to_string(), StepOutcomeDto::Failed));
    assert!(action.steps.last().unwrap().detail.as_deref().unwrap().contains("boom"));
    assert!(!claude.calls().iter().any(|c| c.starts_with("plugin install") || c.starts_with("plugin marketplace add")));
    assert!(files.get(MARKER).is_none());
    assert_eq!(action.status.state, ClaudeCodeStateDto::NotInstalled);
}

#[tokio::test]
async fn cleanup_failures_are_ignored() {
    let (claude, _, setup) = world(Some("2.1.0"), "[]");
    claude.fail("plugin uninstall");
    claude.fail("plugin marketplace remove");
    let action = setup.install().await.unwrap();
    assert!(action.ok);
    let s = steps(&action);
    assert!(s.contains(&("remove-previous-plugin".into(), StepOutcomeDto::Ignored)));
    assert!(s.contains(&("remove-previous-marketplace".into(), StepOutcomeDto::Ignored)));
}

#[tokio::test]
async fn claude_missing_is_a_clean_status_and_install_fails_at_detection() {
    let (claude, files, setup) = world(None, "[]");
    let status = setup.status().await.unwrap();
    assert_eq!(status.state, ClaudeCodeStateDto::ClaudeMissing);
    assert!(status.claude_version.is_none());
    let action = setup.install().await.unwrap();
    assert!(!action.ok);
    assert_eq!(steps(&action), [("detect-claude".to_string(), StepOutcomeDto::Failed)]);
    assert!(files.under(ROOT).is_empty(), "nothing written");
    assert!(claude.calls().iter().all(|c| c == "--version"));
}

#[tokio::test]
async fn install_needs_the_cli_copy_the_hook_calls() {
    let (_, files, setup) = world(Some("2.1.0"), "[]");
    files.remove(Path::new(CLI)).unwrap();
    assert!(!setup.status().await.unwrap().cli_installed);
    let action = setup.install().await.unwrap();
    assert!(!action.ok);
    assert_eq!(steps(&action).last().unwrap(), &("check-cli".to_string(), StepOutcomeDto::Failed));
}

#[tokio::test]
async fn status_states() {
    let (claude, files, setup) = world(Some("2.1.0"), "[]");
    assert_eq!(setup.status().await.unwrap().state, ClaudeCodeStateDto::NotInstalled);
    setup.install().await.unwrap();
    assert_eq!(setup.status().await.unwrap().state, ClaudeCodeStateDto::Installed);
    files.put(MARKER, "0.2.0-h00000000");
    let status = setup.status().await.unwrap();
    assert_eq!(status.state, ClaudeCodeStateDto::Outdated);
    assert_eq!(status.installed_version.as_deref(), Some("0.2.0-h00000000"));
    // Removed with `claude plugin uninstall` outside Lampo.
    *claude.list.lock().unwrap() = "[]".into();
    assert_eq!(setup.status().await.unwrap().state, ClaudeCodeStateDto::NotInstalled);
}

#[tokio::test]
async fn never_installed_refresh_is_a_no_op_without_processes() {
    let (claude, files, setup) = world(Some("2.1.0"), LISTED);
    assert!(setup.find_outdated().is_none());
    assert!(!setup.refresh_if_outdated().await);
    assert!(claude.calls().is_empty(), "no claude process: {:?}", claude.calls());
    assert!(files.under(ROOT).is_empty());
}

#[tokio::test]
async fn up_to_date_refresh_is_a_no_op_without_processes() {
    let (claude, _, setup) = world(Some("2.1.0"), "[]");
    setup.install().await.unwrap();
    claude.calls.lock().unwrap().clear();
    assert!(setup.find_outdated().is_none());
    assert!(!setup.refresh_if_outdated().await);
    assert!(claude.calls().is_empty());
}

#[tokio::test]
async fn outdated_refresh_reinstalls_and_updates_the_marker() {
    let (claude, files, _) = world(Some("2.1.0"), "[]");
    let old = setup_with(claude.clone(), files.clone(), "0.2.0", false);
    assert!(old.install().await.unwrap().ok);
    let old_version = files.get(MARKER).unwrap();

    let new = setup_with(claude.clone(), files.clone(), "0.3.0", false);
    let outdated = new.find_outdated().unwrap();
    assert_eq!(outdated.installed, old_version);
    assert!(new.refresh_if_outdated().await);
    let marker = files.get(MARKER).unwrap();
    assert!(marker.starts_with("0.3.0-h"), "{marker}");
    assert!(new.find_outdated().is_none());
    assert!(new.status().await.unwrap().update_error.is_none());
}

#[tokio::test]
async fn failed_refresh_keeps_the_old_marker_and_surfaces_a_warning() {
    let (claude, files, _) = world(Some("2.1.0"), "[]");
    assert!(setup_with(claude.clone(), files.clone(), "0.2.0", false).install().await.unwrap().ok);
    let old_version = files.get(MARKER).unwrap();
    claude.fail("plugin install");
    let new = setup_with(claude.clone(), files.clone(), "0.3.0", false);
    assert!(!new.refresh_if_outdated().await);
    assert_eq!(files.get(MARKER).unwrap(), old_version);
    let status = new.status().await.unwrap();
    assert!(status.update_error.as_deref().unwrap().starts_with("install-plugin"), "{status:?}");
    // A successful manual update clears the warning.
    claude.failing.lock().unwrap().clear();
    assert!(new.install().await.unwrap().ok);
    assert!(new.status().await.unwrap().update_error.is_none());
}

#[tokio::test]
async fn refresh_does_not_reinstall_a_plugin_removed_outside_lampo() {
    let (claude, files, _) = world(Some("2.1.0"), "[]");
    assert!(setup_with(claude.clone(), files.clone(), "0.2.0", false).install().await.unwrap().ok);
    *claude.list.lock().unwrap() = "[]".into();
    let new = setup_with(claude.clone(), files.clone(), "0.3.0", false);
    claude.calls.lock().unwrap().clear();
    assert!(!new.refresh_if_outdated().await);
    assert!(!claude.calls().iter().any(|c| c.starts_with("plugin install")));
    assert!(files.get(MARKER).is_none(), "forgotten: never touched again");
    assert!(new.find_outdated().is_none());
}

#[tokio::test]
async fn uninstall_unregisters_and_deletes_the_generated_files() {
    let (claude, files, setup) = world(Some("2.1.0"), "[]");
    setup.install().await.unwrap();
    let action = setup.uninstall().await.unwrap();
    assert!(action.ok);
    assert!(claude.calls().contains(&"plugin uninstall lampo@lampo --scope user".to_string()));
    assert!(claude.calls().contains(&"plugin marketplace remove lampo".to_string()));
    assert!(files.under(ROOT).is_empty());
    assert!(files.get(CLI).is_some(), "the CLI copy is not ours to delete");
    assert_eq!(action.status.state, ClaudeCodeStateDto::NotInstalled);
}

#[tokio::test]
async fn isolated_data_root_never_runs_claude() {
    let claude = FakeClaude::new(Some("2.1.0"), LISTED);
    let files = Arc::new(MemFiles::default());
    files.put(MARKER, "0.0.1-h00000000");
    let setup = setup_with(claude.clone(), files.clone(), "0.3.0", true);
    assert_eq!(setup.status().await.unwrap().state, ClaudeCodeStateDto::Isolated);
    assert_eq!(setup.install().await.unwrap_err().code, ErrorCode::Unsupported);
    assert_eq!(setup.uninstall().await.unwrap_err().code, ErrorCode::Unsupported);
    assert!(setup.find_outdated().is_none());
    assert!(!setup.refresh_if_outdated().await);
    assert!(claude.calls().is_empty());
    assert_eq!(files.get(MARKER).as_deref(), Some("0.0.1-h00000000"));
}

#[test]
fn embedded_skills_have_a_valid_core_skill() {
    let skills = embedded_skills();
    let core = skills.iter().find(|s| s.path == "lampo/SKILL.md").expect("claude-plugin/skills/lampo/SKILL.md is embedded");
    for skill in skills.iter().filter(|s| s.path.ends_with("/SKILL.md")) {
        let text = skill.content.replace("\r\n", "\n");
        let front = text.strip_prefix("---\n").and_then(|rest| rest.split_once("\n---\n")).map(|(f, _)| f).unwrap_or_else(|| panic!("{}: frontmatter", skill.path));
        let id = skill.path.split('/').next().unwrap();
        assert!(front.lines().any(|l| l.trim() == format!("name: {id}")), "{}: name must be the folder name", skill.path);
        assert!(front.lines().any(|l| l.starts_with("description: ") && l.len() > 20), "{}: description", skill.path);
    }
    assert!(core.content.contains("lampo agent context --json"));
    assert!(skills.iter().all(|s| !s.path.contains('\\')));
}

#[tokio::test]
async fn startup_updates_an_outdated_plugin_alongside_the_sync() {
    use crate::testing::{FakeAppLocation, World};
    use crate::use_cases::Startup;

    let (claude, files, _) = world(Some("2.1.0"), "[]");
    assert!(setup_with(claude.clone(), files.clone(), "0.2.0", false).install().await.unwrap().ok);
    let new = Arc::new(setup_with(claude.clone(), files.clone(), "0.3.0", false));
    let w = World::with_basics();
    w.hosts_in_sync();
    Startup::new(w.env.clone(), w.settings.clone(), w.shim_setup(), Arc::new(FakeAppLocation::default())).with_claude_code(new.clone()).run().await.unwrap();
    assert!(files.get(MARKER).unwrap().starts_with("0.3.0-h"));
    assert_eq!(w.mysql.calls(), ["prepare:mysql-8.4", "start:mysql-8.4"], "the environment sync ran too");
}
