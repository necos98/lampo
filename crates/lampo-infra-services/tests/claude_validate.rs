//! With Claude Code installed: the generated plugin (with the repository's real skills) passes
//! `claude plugin validate`, run through the real adapter. Only validates: it never adds a
//! marketplace nor installs anything in the user's Claude Code configuration.
//!
//! `cargo test -p lampo-infra-services --test claude_validate -- --ignored --nocapture`

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lampo_domain::claude_code::{self, SkillFile};
use lampo_domain::ports::{ClaudeCodeCli, CommandRunner, ExecutableLocator};
use lampo_domain::PortResult;
use lampo_infra_services::{ClaudeCliRunner, JobObjectSupervisor};

/// The process PATH (enough for a test run from a shell that finds `claude`).
struct EnvPath;
impl ExecutableLocator for EnvPath {
    fn find_all(&self, name: &str) -> PortResult<Vec<PathBuf>> {
        let path = std::env::var_os("PATH").unwrap_or_default();
        Ok(std::env::split_paths(&path).flat_map(|dir| ["exe", "cmd", "bat"].map(|ext| dir.join(format!("{name}.{ext}")))).filter(|p| p.is_file()).collect())
    }
}

fn repo_skills(dir: &Path, root: &Path, out: &mut Vec<SkillFile>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            repo_skills(&path, root, out);
        } else {
            let relative = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push(SkillFile::new(relative, std::fs::read_to_string(&path).unwrap()));
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Claude Code (`claude`) installed"]
async fn claude_validate_accepts_the_generated_plugin() {
    let skills_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../claude-plugin/skills");
    let mut skills = Vec::new();
    repo_skills(&skills_root, &skills_root, &mut skills);
    assert!(skills.iter().any(|s| s.path == "lampo/SKILL.md"));

    let icon = std::fs::read(skills_root.join("../icon.png")).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("agent plugins").join("claude-code");
    let bundle = claude_code::bundle(env!("CARGO_PKG_VERSION"), r"C:\Users\me\AppData\Local\Lampo\bin\lampo.exe", &skills, Some(icon.as_slice()));
    for file in &bundle.files {
        let path = root.join(file.path.replace('/', "\\"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &file.content).unwrap();
    }
    for asset in &bundle.assets {
        std::fs::write(root.join(asset.path.replace('/', "\\")), &asset.bytes).unwrap();
    }

    let runner: Arc<dyn CommandRunner> = Arc::new(JobObjectSupervisor::new());
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let claude = ClaudeCliRunner::new(runner, Arc::new(EnvPath), home);
    let version = claude.detect().await.unwrap().expect("claude is installed");
    println!("claude: {version} ({:?})", claude.locate());

    // Only the two validation steps of the install flow.
    for step in claude_code::install_steps(&root.display().to_string()).into_iter().filter(|s| s.id.starts_with("validate-")) {
        let out = claude.run(&step.args, Duration::from_secs(60)).await.unwrap();
        println!("{} {:?}\n{}{}", step.id, out.exit_code, out.stdout, out.stderr);
        assert!(out.success(), "{} failed: {}{}", step.id, out.stdout, out.stderr);
        assert!(!out.stdout.contains("ICON_MISSING") && !out.stderr.contains("ICON_MISSING"), "{} still misses the icon", step.id);
    }
}
