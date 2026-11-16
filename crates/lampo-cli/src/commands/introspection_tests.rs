//! `which` (the shim's resolution and invocation, explained) and `completions` (the script of the
//! real clap definition).

use std::sync::atomic::Ordering;

use clap::Parser;
use serde_json::Value;

use crate::cli::Cli;
use crate::testing::*;

fn parse(out: &str) -> Value {
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("stdout is not exactly one JSON document ({e}): {out:?}"))
}

#[tokio::test]
async fn which_json_is_the_dto_and_never_starts_the_app() {
    let fake = Fake::new();
    fake.app_running.store(false, Ordering::SeqCst);
    let (code, out, err) = run_cli(&fake, &["which", "mysql", "--json"]).await;
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(parse(&out), which_json("mysql"));
    assert_eq!(fake.ensure_calls.load(Ordering::SeqCst), 0);
    let (request, tool) = fake.which_requests.lock().unwrap().last().cloned().unwrap();
    assert_eq!((request.cwd.as_str(), tool.as_str()), (r"C:\work\shop", "mysql"));
}

#[tokio::test]
async fn which_passes_the_project_flag_and_prints_the_binary_and_the_reason() {
    let fake = Fake::new();
    let (code, out, _) = run_cli(&fake, &["which", "php", "--project", "shop"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("php: C:\\php\\8.3\\php.exe\n"), "{out}");
    assert!(out.contains("reason:  the current folder is inside"), "{out}");
    let (request, _) = fake.which_requests.lock().unwrap().last().cloned().unwrap();
    assert_eq!(request.explicit_site.as_deref(), Some("shop"));
}

#[tokio::test]
async fn which_of_a_tool_the_site_lacks_is_exit_10() {
    let fake = Fake::new();
    let (code, out, _) = run_cli(&fake, &["which", "redis-cli", "--json"]).await;
    assert_eq!(code, 10);
    assert_eq!(parse(&out)["error"]["code"], "package-missing");
}

#[test]
fn which_accepts_only_shim_names_in_any_case() {
    for tool in ["php", "composer", "mysql", "mysqldump", "redis-cli", "PHP"] {
        assert!(Cli::try_parse_from(["lampo", "which", tool]).is_ok(), "{tool}");
    }
    for bad in [&["lampo", "which", "node"][..], &["lampo", "which"]] {
        assert!(Cli::try_parse_from(bad).is_err(), "{bad:?}");
    }
}

#[tokio::test]
async fn completions_print_a_script_that_knows_every_command() {
    let fake = Fake::new();
    let markers = [("powershell", "Register-ArgumentCompleter"), ("bash", "_lampo()"), ("zsh", "#compdef lampo"), ("fish", "complete -c lampo")];
    for (shell, marker) in markers {
        let (code, out, err) = run_cli(&fake, &["completions", shell]).await;
        assert_eq!((code, err.as_str()), (0, ""), "{shell}");
        assert!(out.contains(marker), "{shell}: {out}");
        for word in ["which", "worktree", "redis-cli", "--project"] {
            assert!(out.contains(word), "{shell} lacks {word}");
        }
    }
    assert_eq!(fake.which_requests.lock().unwrap().len(), 0);
}

#[test]
fn completions_reject_json_and_unknown_shells() {
    assert!(Cli::try_parse_from(["lampo", "completions", "bash", "--json"]).is_err());
    assert!(Cli::try_parse_from(["lampo", "completions", "cmd"]).is_err());
    assert!(Cli::try_parse_from(["lampo", "completions"]).is_err());
}
