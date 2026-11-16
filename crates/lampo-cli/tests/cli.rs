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
        &["project", "detect"],
        &["project", "create"],
        &["project", "activate"],
        &["project", "deactivate"],
        &["project", "set-php"],
        &["project", "set"],
        &["project", "rename"],
        &["project", "remove"],
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
        &["package", "uninstall"],
        &["package", "extensions"],
        &["db"],
        &["db", "list"],
        &["db", "create"],
        &["db", "drop"],
        &["db", "import"],
        &["db", "export"],
        &["exec"],
        &["env"],
        &["which"],
        &["completions"],
        &["doctor"],
        &["agent"],
        &["agent", "context"],
        &["shims"],
        &["shims", "status"],
        &["shims", "install"],
        &["shims", "uninstall"],
        &["settings"],
        &["settings", "get"],
        &["settings", "set"],
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

/// `project detect` runs in-process with the real detectors and writes nothing: a Laravel folder
/// with a Compose file, the current folder by default, and a missing folder (exit 4).
#[test]
fn project_detect_reads_a_real_folder_without_the_app() {
    let home = tempfile::tempdir().unwrap();
    let sites = tempfile::tempdir().unwrap();
    let shop = sites.path().join("shop");
    std::fs::create_dir(&shop).unwrap();
    std::fs::write(shop.join("artisan"), "").unwrap();
    std::fs::write(shop.join("composer.json"), r#"{"require":{"php":"^8.2","laravel/framework":"^11.0"}}"#).unwrap();
    std::fs::write(shop.join("compose.yaml"), "services:\n  redis:\n    image: redis:7\n").unwrap();
    let detect = |cwd: &std::path::Path, args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_lampo"))
            .args(["project", "detect"])
            .args(args)
            .args(["--json", "--no-start"])
            .env("LAMPO_HOME", home.path())
            .env_remove("LAMPO_PROJECT")
            .current_dir(cwd)
            .output()
            .unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
        (out.status.code().unwrap(), doc)
    };

    let (code, doc) = detect(sites.path(), &["shop"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["folder"], shop.display().to_string());
    assert_eq!((doc["projectType"].as_str(), doc["presetId"].as_str(), doc["phpVersion"].as_str()), (Some("laravel"), Some("laravel"), Some("8.2")));
    assert_eq!((doc["suggestedId"].as_str(), doc["alreadyRegistered"].as_str()), (Some("shop"), None));
    assert!(doc["services"].as_array().unwrap().contains(&serde_json::json!("redis")), "{doc}");

    let (code, doc) = detect(&shop, &[]);
    assert_eq!((code, doc["folder"].as_str()), (0, Some(shop.display().to_string().as_str())));

    let (code, doc) = detect(sites.path(), &["missing"]);
    assert_eq!((code, doc["error"]["code"].as_str()), (4, Some("validation")));

    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0, "detection must not write to the data root");
}

/// `package extensions` runs in-process (no app, `--no-start`): on an empty data root the PHP is
/// not installed (exit 10), nothing resolves without `--php` (exit 6), and nothing is written.
/// `package uninstall` without `--yes` is refused before any app contact (exit 4).
#[test]
fn package_extensions_and_uninstall_guard_without_the_app() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_lampo"))
            .args(args)
            .args(["--json", "--no-start"])
            .env("LAMPO_HOME", home.path())
            .env_remove("LAMPO_PROJECT")
            .current_dir(cwd.path())
            .output()
            .unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
        (out.status.code().unwrap(), doc["error"]["code"].as_str().map(str::to_string))
    };
    assert_eq!(run(&["package", "extensions", "--php", "8.3"]), (10, Some("package-missing".into())));
    assert_eq!(run(&["package", "extensions"]), (6, Some("project-not-resolved".into())));
    assert_eq!(run(&["package", "uninstall", "php", "8.3"]), (4, Some("validation".into())));
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0, "read-only commands must not write to the data root");
}
