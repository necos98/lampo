//! Behaviour of the command layer against fakes: arguments -> control requests, JSON output
//! shapes (as exact documents), exit codes, and the "one JSON document on stdout" rule.

use std::sync::atomic::Ordering;

use lampo_application::context::ElevationPolicy;
use lampo_application::control::ControlRequest;
use lampo_application::dto::*;
use lampo_application::{AppError, ErrorCode};
use serde_json::{Value, json};

use crate::testing::*;

fn parse(out: &str) -> Value {
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("stdout is not exactly one JSON document ({e}): {out:?}"))
}

#[tokio::test]
async fn status_without_the_app_uses_the_offline_view_and_never_starts_it() {
    let fake = Fake::new();
    fake.app_running.store(false, Ordering::SeqCst);
    let (code, out, _) = run_cli(&fake, &["status", "--json"]).await;
    assert_eq!(code, 0);
    let doc = parse(&out);
    assert_eq!(doc["appRunning"], false);
    assert_eq!(doc["services"][0]["state"], "unknown");
    assert_eq!(fake.ensure_calls.load(Ordering::SeqCst), 0);
    assert!(fake.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn status_with_the_app_asks_it() {
    let fake = Fake::new();
    fake.reply(Ok(json!({"appRunning": true, "services": [], "projects": [], "worktreeCount": 0})));
    let (code, out, _) = run_cli(&fake, &["status", "--json"]).await;
    assert_eq!(code, 0);
    assert_eq!(parse(&out)["appRunning"], true);
    assert_eq!(fake.last_request().0, ControlRequest::Status);
}

#[tokio::test]
async fn project_list_json_is_the_dto_array() {
    let fake = Fake::new();
    let (code, out, err) = run_cli(&fake, &["project", "list", "--json"]).await;
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(parse(&out), json!([summary_json()]));
}

#[tokio::test]
async fn project_list_human_is_a_table() {
    let fake = Fake::new();
    let (_, out, _) = run_cli(&fake, &["project", "list"]).await;
    assert!(out.starts_with("ID"), "{out}");
    assert!(out.contains("http://127.0.0.1:8101"));
}

#[tokio::test]
async fn project_info_defaults_to_the_resolved_project_and_reports_not_found() {
    let fake = Fake::new();
    let (code, out, _) = run_cli(&fake, &["project", "info", "--json"]).await;
    assert_eq!(code, 0);
    assert_eq!(parse(&out)["id"], "shop");

    let (code, out, err) = run_cli(&fake, &["project", "info", "nope", "--json"]).await;
    assert_eq!(code, 3);
    let doc = parse(&out);
    assert_eq!(doc["error"]["code"], "not-found");
    assert!(doc["error"]["hint"].as_str().unwrap().contains("lampo project list"));
    assert!(err.contains("project `nope` not found"));
}

#[tokio::test]
async fn project_info_outside_a_project_is_exit_6() {
    let fake = Fake::new();
    fake.resolves_to(None);
    let (code, out, _) = run_cli(&fake, &["project", "info", "--json"]).await;
    assert_eq!(code, 6);
    assert_eq!(parse(&out)["error"]["code"], "project-not-resolved");
}

#[tokio::test]
async fn project_create_defaults_to_no_elevation_addressing_and_an_absolute_root() {
    let fake = Fake::new();
    fake.reply(Ok(project_json()));
    let (code, out, _) = run_cli(&fake, &["project", "create", "--name", "Shop", "--php", "8.3", "--activate", "--json"]).await;
    assert_eq!(code, 0);
    assert_eq!(parse(&out)["id"], "shop");
    let (request, context) = fake.last_request();
    let ControlRequest::ProjectCreate(r) = request else { panic!("wrong request") };
    assert_eq!(r.root, r"C:\work\shop");
    assert_eq!(r.addressing, AddressingDto::NoElevation);
    assert_eq!((r.name.as_deref(), r.php_version.as_deref(), r.activate), (Some("Shop"), Some("8.3"), true));
    assert_eq!(context.elevation, ElevationPolicy::Deny);
}

#[tokio::test]
async fn project_create_creates_the_database_unless_no_database() {
    let fake = Fake::new();
    let mut reply = project_json();
    reply["databaseSetup"] = json!({ "name": "shop", "status": "created", "message": null });
    fake.reply(Ok(reply));
    let (code, out, err) = run_cli(&fake, &["project", "create", "--json"]).await;
    assert_eq!(code, 0);
    let (ControlRequest::ProjectCreate(r), _) = fake.last_request() else { panic!("wrong request") };
    assert!(r.create_database);
    let doc = parse(&out);
    assert_eq!((doc["id"].as_str(), doc["databaseSetup"]["status"].as_str()), (Some("shop"), Some("created")));
    assert!(err.is_empty());

    fake.reply(Ok(project_json()));
    let (code, _, _) = run_cli(&fake, &["project", "create", "--no-database"]).await;
    assert_eq!(code, 0);
    let (ControlRequest::ProjectCreate(r), _) = fake.last_request() else { panic!("wrong request") };
    assert!(!r.create_database);
}

#[tokio::test]
async fn a_failed_database_setup_is_a_warning_not_an_error() {
    let fake = Fake::new();
    let mut reply = project_json();
    reply["databaseSetup"] = json!({ "name": "shop", "status": "failed", "message": "MySQL 8.4 did not start" });
    fake.reply(Ok(reply));
    let (code, out, err) = run_cli(&fake, &["project", "create"]).await;
    assert_eq!(code, 0, "the project was created");
    assert!(out.contains("database setup: failed"), "{out}");
    assert!(err.contains("warning: the database `shop` was not created: MySQL 8.4 did not start"), "{err}");
}

#[tokio::test]
async fn hosts_name_uses_standard_addressing_and_allow_elevation_is_forwarded() {
    let fake = Fake::new();
    fake.reply(Ok(project_json()));
    let (code, _, _) = run_cli(&fake, &["project", "create", r"D:\x\app", "--hosts-name", "--allow-elevation"]).await;
    assert_eq!(code, 0);
    let (ControlRequest::ProjectCreate(r), context) = fake.last_request() else { panic!("wrong request") };
    assert_eq!(r.root, r"D:\x\app");
    assert_eq!(r.addressing, AddressingDto::Standard);
    assert_eq!(context.elevation, ElevationPolicy::Allow);
}

#[tokio::test]
async fn elevation_required_is_exit_7_with_a_json_error() {
    let fake = Fake::new();
    fake.reply(Err(AppError::new(ErrorCode::ElevationRequired, "the hosts file must change")));
    let (code, out, _) = run_cli(&fake, &["project", "activate", "--json"]).await;
    assert_eq!(code, 7);
    let doc = parse(&out);
    assert_eq!(doc["error"]["code"], "elevation-required");
    assert_eq!(doc["error"]["exitCode"], 7);
    assert!(doc["error"]["hint"].as_str().unwrap().contains("--allow-elevation"));
}

#[tokio::test]
async fn mutating_commands_start_the_app_unless_no_start() {
    let fake = Fake::new();
    fake.app_running.store(false, Ordering::SeqCst);
    fake.reply(Ok(json!({"started": ["apache"], "stopped": [], "restarted": [], "hostsFileUpdated": false, "warnings": []})));
    let (code, out, _) = run_cli(&fake, &["up", "--json"]).await;
    assert_eq!(code, 0);
    assert_eq!(fake.ensure_calls.load(Ordering::SeqCst), 1);
    assert_eq!(parse(&out)["started"], json!(["apache"]));

    let fake = Fake::new();
    fake.app_running.store(false, Ordering::SeqCst);
    let (code, out, _) = run_cli(&fake, &["up", "--no-start", "--json"]).await;
    assert_eq!(code, 9);
    assert_eq!(parse(&out)["error"]["code"], "app-unavailable");
    assert_eq!(fake.ensure_calls.load(Ordering::SeqCst), 0);
    assert!(fake.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn app_that_cannot_start_is_exit_9() {
    let fake = Fake::new();
    fake.app_running.store(false, Ordering::SeqCst);
    *fake.ensure_result.lock().unwrap() = Err(AppError::new(ErrorCode::AppUnavailable, "the Lampo app did not start in time"));
    let (code, _, err) = run_cli(&fake, &["down"]).await;
    assert_eq!(code, 9);
    assert!(err.contains("did not start"));
}

#[tokio::test]
async fn service_names_are_forwarded() {
    let fake = Fake::new();
    fake.reply(Ok(json!({"started": [], "stopped": [], "restarted": ["mysql-8.4", "redis"], "hostsFileUpdated": false, "warnings": []})));
    let (code, out, _) = run_cli(&fake, &["restart", "mysql-8.4", "redis"]).await;
    assert_eq!(code, 0);
    assert!(out.contains("restarted: mysql-8.4, redis"));
    assert_eq!(fake.last_request().0, ControlRequest::ServicesRestart(ServicesRequest { services: vec!["mysql-8.4".into(), "redis".into()] }));
}

#[tokio::test]
async fn progress_goes_to_stderr_and_stdout_stays_one_document() {
    let fake = Fake::new();
    fake.progress_to_send.lock().unwrap().push(dto(json!({"stage": "downloading", "done": 5, "total": 10, "subject": "php 8.3.12"})));
    fake.reply(Ok(json!({"id": {"kind": "php", "version": "8.3.12", "phpVersion": null}, "path": "C:\\p", "usedBy": []})));
    let (code, out, err) = run_cli(&fake, &["package", "install", "php", "8.3", "--json"]).await;
    assert_eq!(code, 0);
    assert_eq!(parse(&out)["id"]["version"], "8.3.12");
    assert!(err.contains("downloading php 8.3.12"), "{err}");
    let (ControlRequest::PackageInstall(r), _) = fake.last_request() else { panic!("wrong request") };
    assert_eq!((r.kind.as_str(), r.version, r.series.as_deref()), ("php", None, Some("8.3")));
}

#[tokio::test]
async fn package_list_variants() {
    let fake = Fake::new();
    let (_, out, _) = run_cli(&fake, &["package", "list", "--json"]).await;
    assert_eq!(parse(&out)[0]["installed"], false);
    let (_, out, _) = run_cli(&fake, &["package", "list", "--installed", "--kind", "mysql", "--json"]).await;
    assert_eq!(parse(&out), json!([]));
}

#[tokio::test]
async fn db_drop_requires_yes_and_sends_nothing_without_it() {
    let fake = Fake::new();
    let (code, out, _) = run_cli(&fake, &["db", "drop", "shop", "--json"]).await;
    assert_eq!(code, 4);
    assert_eq!(parse(&out)["error"]["code"], "validation");
    assert!(fake.sent.lock().unwrap().is_empty());

    fake.reply(Ok(json!("shop")));
    let (code, out, _) = run_cli(&fake, &["db", "drop", "shop", "--yes", "--json"]).await;
    assert_eq!(code, 0);
    assert_eq!(parse(&out), json!({"site": "shop", "action": "dropped", "database": "shop", "file": null}));
}

#[tokio::test]
async fn db_commands_target_the_resolved_site_and_absolute_files() {
    let fake = Fake::new();
    fake.reply(Ok(Value::Null));
    let (code, _, _) = run_cli(&fake, &["db", "import", "dump.sql", "other"]).await;
    assert_eq!(code, 0);
    let (ControlRequest::DbImport(r), _) = fake.last_request() else { panic!("wrong request") };
    assert_eq!((r.site.as_str(), r.name.as_deref(), r.file.as_str()), ("shop", Some("other"), r"C:\work\shop\dump.sql"));

    fake.resolves_to(None);
    let (code, _, _) = run_cli(&fake, &["db", "list"]).await;
    assert_eq!(code, 6);
}

#[tokio::test]
async fn worktree_up_and_down() {
    let fake = Fake::new();
    fake.reply(Err(AppError::new(ErrorCode::Validation, "stop here")));
    let (code, _, _) = run_cli(&fake, &["worktree", "up", "--db", "clone:shop", "--parent", "shop"]).await;
    assert_eq!(code, 4);
    let (ControlRequest::WorktreeUp(r), _) = fake.last_request() else { panic!("wrong request") };
    assert_eq!(r.database, DatabaseModeDto::Clone { source: "shop".into() });
    assert_eq!(r.project_id.as_deref(), Some("shop"));

    let (code, out, _) = run_cli(&fake, &["worktree", "up", "--db", "bogus", "--json"]).await;
    assert_eq!(code, 4);
    assert!(parse(&out)["error"]["message"].as_str().unwrap().contains("--db"));

    fake.reply(Ok(json!({"site": "shop@x", "databaseDropped": "shop_x"})));
    let (code, out, _) = run_cli(&fake, &["worktree", "down", "shop@x", "--drop-database", "--json"]).await;
    assert_eq!(code, 0);
    assert_eq!(parse(&out)["databaseDropped"], "shop_x");
    let (ControlRequest::WorktreeDown(r), _) = fake.last_request() else { panic!("wrong request") };
    assert_eq!((r.target.as_str(), r.drop_database), ("shop@x", true));

    fake.reply(Ok(json!({"dryRun": true, "removed": ["shop@old"], "droppedDatabases": []})));
    let (code, out, _) = run_cli(&fake, &["worktree", "prune", "--dry-run", "-p", "shop@old"]).await;
    assert_eq!(code, 0);
    assert!(out.contains("Would remove 1"));
    let (ControlRequest::WorktreePrune(r), _) = fake.last_request() else { panic!("wrong request") };
    assert_eq!((r.project_id.as_deref(), r.dry_run), (Some("shop"), true));
}

#[tokio::test]
async fn logs_lists_sources_and_tails_plain_or_json() {
    let fake = Fake::new();
    let (_, out, _) = run_cli(&fake, &["logs", "--json"]).await;
    assert_eq!(parse(&out)[0]["id"], "access:shop");

    let (code, out, _) = run_cli(&fake, &["logs", "apache", "-n", "2"]).await;
    assert_eq!((code, out.as_str()), (0, "one\ntwo\n"));

    let (_, out, _) = run_cli(&fake, &["logs", "error", "--since", "10m", "--json"]).await;
    assert_eq!(parse(&out)["nextOffset"], 8);
    let request = fake.tail_requests.lock().unwrap().last().cloned().unwrap();
    assert_eq!((request.source.as_str(), request.since_unix_ms), ("error:shop", Some(400_000)));
}

#[tokio::test]
async fn logs_rejects_a_bad_since_value() {
    let fake = Fake::new();
    let (code, out, _) = run_cli(&fake, &["logs", "apache", "--since", "soon", "--json"]).await;
    assert_eq!(code, 4);
    assert!(parse(&out)["error"]["message"].as_str().unwrap().contains("--since"));
}

#[tokio::test]
async fn logs_follow_streams_ndjson_until_interrupted_and_exits_zero() {
    let fake = Fake::new();
    fake.follow_reads.lock().unwrap().push_back(dto(json!({"source": "apache", "file": "f", "lines": [{"text": "three", "atUnixMs": 3}], "nextOffset": 20})));
    let (code, out, _) = run_cli(&fake, &["logs", "apache", "--follow", "--json"]).await;
    assert_eq!(code, 0);
    let lines: Vec<Value> = out.lines().map(|l| serde_json::from_str(l).expect("one JSON document per line")).collect();
    assert_eq!(lines.iter().map(|l| l["text"].as_str().unwrap().trim()).collect::<Vec<_>>(), ["one", "two", "three"]);
    assert_eq!(fake.follow_offsets.lock().unwrap().as_slice(), [8]);
}

#[tokio::test]
async fn exec_returns_the_childs_exit_code_and_passes_the_toolchain() {
    let fake = Fake::new();
    *fake.spawn_result.lock().unwrap() = Ok(42);
    let (code, out, _) = run_cli(&fake, &["exec", "--json", "--", "composer", "install", "--no-interaction"]).await;
    assert_eq!(code, 42);
    assert_eq!(out, "", "exec prints nothing of its own");
    let spec = fake.spawned.lock().unwrap()[0].clone();
    assert_eq!((spec.program.as_str(), spec.args.as_slice()), ("composer", ["install".to_string(), "--no-interaction".to_string()].as_slice()));
    assert_eq!(spec.path_prepend, [r"C:\bin", r"C:\php\8.3"]);
    assert_eq!(spec.vars["PHPRC"], r"C:\ini");
}

#[tokio::test]
async fn exec_of_a_missing_program_is_not_found() {
    let fake = Fake::new();
    *fake.spawn_result.lock().unwrap() = Err(crate::ports::SpawnError::NotFound("nope".into()));
    let (code, _, err) = run_cli(&fake, &["exec", "nope"]).await;
    assert_eq!(code, 3);
    assert!(err.contains("`nope` not found"));
}

#[tokio::test]
async fn env_renders_per_shell_and_as_json() {
    let fake = Fake::new();
    let (_, out, _) = run_cli(&fake, &["env", "--shell", "bash"]).await;
    assert_eq!(out, "export PATH='/c/bin:/c/php/8.3'\":$PATH\"\nexport LAMPO_PROJECT='shop'\nexport PHPRC='C:\\ini'\n");
    let (_, out, _) = run_cli(&fake, &["env", "--shell", "powershell"]).await;
    assert!(out.starts_with("$env:PATH = 'C:\\bin;C:\\php\\8.3' + ';' + $env:PATH\n"), "{out}");
    let (_, out, _) = run_cli(&fake, &["env", "--shell", "cmd"]).await;
    assert!(out.starts_with("set \"PATH=C:\\bin;C:\\php\\8.3;%PATH%\"\n"), "{out}");
    let (_, out, _) = run_cli(&fake, &["env", "--json"]).await;
    assert_eq!(parse(&out), toolchain_json());
}

#[tokio::test]
async fn doctor_exits_14_when_there_are_problems() {
    let fake = Fake::new();
    let (code, _, _) = run_cli(&fake, &["doctor", "--json"]).await;
    assert_eq!(code, 0);
    fake.doctor_ok.store(false, Ordering::SeqCst);
    let (code, out, _) = run_cli(&fake, &["doctor", "--json"]).await;
    assert_eq!(code, 14);
    assert_eq!(parse(&out)["ok"], false, "the report is still the single JSON document");
}

#[tokio::test]
async fn agent_context_json_adds_the_cli_guide_to_the_dto() {
    let fake = Fake::new();
    let (code, out, _) = run_cli(&fake, &["agent", "context", "--json"]).await;
    assert_eq!(code, 0);
    let doc = parse(&out);
    let expected = agent_context_json();
    for (key, value) in expected.as_object().unwrap() {
        assert_eq!(&doc[key], value, "{key}");
    }
    assert_eq!(doc["cli"]["exitCodes"].as_array().unwrap().len(), 16);
    assert_eq!(doc["cli"]["exitCodes"][7]["code"], 7);
    assert!(doc["cli"]["jsonErrorShape"]["error"]["hint"].is_string());
}

#[tokio::test]
async fn agent_context_human_is_complete() {
    let fake = Fake::new();
    let (_, out, _) = run_cli(&fake, &["agent", "context"]).await;
    for needle in [
        "http://127.0.0.1:8101",
        "C:\\php\\8.3\\php.exe",
        "## Database",
        "## Logs",
        "## Commands",
        "lampo status",
        "## Hints",
        "lampo worktree up",
        "## Exit codes",
        " 7  needs the hosts-file UAC",
        "--allow-elevation",
        "## JSON errors",
    ] {
        assert!(out.contains(needle), "missing {needle:?} in:\n{out}");
    }
}

#[tokio::test]
async fn shims_install_then_status() {
    let fake = Fake::new();
    let (_, out, _) = run_cli(&fake, &["shims", "status", "--json"]).await;
    assert_eq!(parse(&out)["installed"], false);
    let (_, out, _) = run_cli(&fake, &["shims", "install", "--json"]).await;
    assert_eq!(parse(&out)["installed"], true);
}

#[tokio::test]
async fn every_json_command_prints_exactly_one_document() {
    let fake = Fake::new();
    fake.app_running.store(false, Ordering::SeqCst);
    let commands: &[&[&str]] = &[
        &["status"],
        &["project", "list"],
        &["project", "info"],
        &["worktree", "list"],
        &["logs"],
        &["logs", "apache"],
        &["package", "list"],
        &["package", "list", "--installed"],
        &["env"],
        &["doctor"],
        &["agent", "context"],
        &["shims", "status"],
    ];
    for command in commands {
        let mut args = command.to_vec();
        args.push("--json");
        let (code, out, _) = run_cli(&fake, &args).await;
        assert_eq!(code, 0, "{command:?}");
        parse(&out);
    }
}

#[test]
fn the_applications_command_catalog_matches_the_clap_surface() {
    let root = crate::cli::build_command();
    for entry in lampo_application::use_cases::agent::cli_command_catalog() {
        let mut words = entry.name.split_whitespace();
        assert_eq!(words.next(), Some("lampo"));
        let mut command = &root;
        for word in words {
            command = command.find_subcommand(word).unwrap_or_else(|| panic!("`{}` is advertised to agents but is not a command", entry.name));
        }
    }
}

#[test]
fn every_command_is_advertised_to_agents() {
    fn leaves(prefix: &str, command: &clap::Command, out: &mut Vec<String>) {
        let subs: Vec<_> = command.get_subcommands().filter(|c| c.get_name() != "help").collect();
        if subs.is_empty() {
            out.push(prefix.to_string());
        }
        for sub in subs {
            leaves(&format!("{prefix} {}", sub.get_name()), sub, out);
        }
    }
    let mut all = Vec::new();
    leaves("lampo", &crate::cli::build_command(), &mut all);
    let advertised: Vec<String> = lampo_application::use_cases::agent::cli_command_catalog().into_iter().map(|c| c.name).collect();
    let missing: Vec<&String> = all.iter().filter(|c| !advertised.contains(c)).collect();
    assert!(missing.is_empty(), "not in the agent command catalog: {missing:?}");
}

#[tokio::test]
async fn project_new_scaffolds_in_the_app_then_registers_the_folder() {
    let fake = Fake::new();
    fake.reply(Ok(json!({ "folder": r"C:\work\shop\blog", "phpVersion": null })));
    fake.reply(Ok(project_json()));
    let (code, out, _) = run_cli(&fake, &["project", "new", "--git", "https://github.com/acme/blog.git", "--branch", "main", "--activate", "--json"]).await;
    assert_eq!(code, 0, "{out}");
    assert_eq!(parse(&out)["id"], "shop");
    let sent = fake.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    let ControlRequest::ProjectScaffold(s) = &sent[0].0 else { panic!("wrong request: {:?}", sent[0].0) };
    assert_eq!(s.parent_dir, r"C:\work\shop");
    assert_eq!(s.folder_name, None);
    assert_eq!(s.source, ScaffoldSourceDto::GitClone { url: "https://github.com/acme/blog.git".into(), branch: Some("main".into()) });
    let ControlRequest::ProjectCreate(r) = &sent[1].0 else { panic!("wrong request") };
    assert_eq!((r.root.as_str(), r.activate, r.php_version.as_deref()), (r"C:\work\shop\blog", true, None));
}

#[tokio::test]
async fn project_new_laravel_maps_the_options_and_can_skip_registration() {
    let fake = Fake::new();
    fake.reply(Ok(json!({ "folder": r"D:\sites\shop", "phpVersion": "8.4" })));
    let args = ["project", "new", "--laravel", "shop", "--in", r"D:\sites", "--starter-kit", "svelte", "--phpunit", "--init-git", "--npm", "--php", "8.4", "--no-register", "--json"];
    let (code, out, _) = run_cli(&fake, &args).await;
    assert_eq!(code, 0, "{out}");
    assert_eq!(parse(&out)["folder"], r"D:\sites\shop");
    let (ControlRequest::ProjectScaffold(s), _) = fake.last_request() else { panic!("wrong request") };
    assert_eq!(s.folder_name.as_deref(), Some("shop"));
    assert_eq!(s.parent_dir, r"D:\sites");
    assert_eq!(s.source, ScaffoldSourceDto::Laravel { php_version: Some("8.4".into()), starter_kit: StarterKitDto::Svelte, testing: TestingFrameworkDto::Phpunit, git: true, npm: true });
    assert_eq!(fake.sent.lock().unwrap().len(), 1, "not registered");

    // A source is required, and only one; Laravel options need --laravel.
    for bad in [&["project", "new"][..], &["project", "new", "--git", "u", "--laravel", "x"], &["project", "new", "--git", "u", "--phpunit"]] {
        let argv = std::iter::once("lampo").chain(bad.iter().copied());
        assert!(crate::cli::build_command().try_get_matches_from(argv).is_err(), "{bad:?}");
    }
}
