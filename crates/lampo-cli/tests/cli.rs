//! The real `lampo` binary: help rendering and usage errors (none of these need the app or any
//! adapter).

use std::process::Command;

fn lampo(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_lampo")).args(args).output().unwrap();
    (out.status.code().unwrap(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn root_help_documents_agents_json_and_exit_codes() {
    let (code, out, _) = lampo(&["--help"]);
    assert_eq!(code, 0);
    for needle in
        ["lampo agent context --json", "Exit codes:", " 7  needs the hosts-file UAC prompt", "14  doctor found problems", "--json", "--allow-elevation", "--no-start", "exec", "env"]
    {
        assert!(out.contains(needle), "missing {needle:?} in:\n{out}");
    }
}

#[test]
fn every_subcommand_renders_help() {
    let commands: &[&[&str]] = &[
        &["status"],
        &["project"],
        &["project", "list"],
        &["project", "info"],
        &["project", "create"],
        &["project", "activate"],
        &["project", "deactivate"],
        &["project", "set-php"],
        &["project", "set"],
        &["worktree"],
        &["worktree", "up"],
        &["worktree", "down"],
        &["worktree", "list"],
        &["worktree", "prune"],
        &["up"],
        &["down"],
        &["restart"],
        &["logs"],
        &["package"],
        &["package", "list"],
        &["package", "install"],
        &["db"],
        &["db", "list"],
        &["db", "create"],
        &["db", "drop"],
        &["db", "import"],
        &["db", "export"],
        &["exec"],
        &["env"],
        &["doctor"],
        &["agent"],
        &["agent", "context"],
        &["shims"],
        &["shims", "status"],
        &["shims", "install"],
    ];
    for command in commands {
        let mut args = command.to_vec();
        args.push("--help");
        let (code, out, err) = lampo(&args);
        assert_eq!(code, 0, "{command:?}: {err}");
        assert!(out.contains("Usage:"), "{command:?}: {out}");
    }
}

#[test]
fn usage_errors_exit_2_and_are_json_on_request() {
    let (code, out, err) = lampo(&["bogus"]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(err.contains("bogus"));

    let (code, out, _) = lampo(&["bogus", "--json"]);
    assert_eq!(code, 2);
    let doc: serde_json::Value = serde_json::from_str(out.trim()).expect("one JSON document");
    assert_eq!(doc["error"]["code"], "usage");
    assert_eq!(doc["error"]["exitCode"], 2);
    assert!(doc["error"]["hint"].as_str().unwrap().contains("--help"));
}

#[test]
fn json_flag_after_double_dash_belongs_to_the_command() {
    let (code, out, _) = lampo(&["exec", "--", "tool", "--json", "--nonsense"]);
    // Parses fine (the failure, if any, happens later when resolving the site), and is not a usage error.
    assert_ne!(code, 2, "{out}");
}

#[test]
fn version_is_printed() {
    let (code, out, _) = lampo(&["--version"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("lampo "));
}

/// The hook never disturbs a Claude Code session: outside any project, or when nothing can be
/// resolved (empty isolated data root, no default PHP), it prints nothing and exits 0.
#[test]
fn agent_context_hook_is_silent_outside_projects() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lampo"))
        .args(["agent", "context", "--hook"])
        .env("LAMPO_HOME", home.path())
        .env_remove("LAMPO_PROJECT")
        .current_dir(cwd.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let payload = format!(r#"{{"cwd":{:?},"hook_event_name":"SessionStart"}}"#, cwd.path().display().to_string());
        child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
    }
    let started = std::time::Instant::now();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "{}", String::from_utf8_lossy(&out.stdout));
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}
