//! `StreamingCommandRunner`: live lines (split at LF and CR), environment, exit codes, and
//! cancellation killing the whole process tree.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lampo_domain::PortError;
use lampo_domain::ports::{CommandSpec, StreamingCommandRunner};
use lampo_domain::progress::CancelFlag;
use lampo_domain::service::LogStream;
use lampo_infra_services::JobObjectSupervisor;

mod common;
use common::{ProcHandle, children_named};

fn cmd(script: &str) -> CommandSpec {
    CommandSpec { program: "C:\\Windows\\System32\\cmd.exe".into(), args: vec!["/d".into(), "/s".into(), "/c".into(), script.into()], ..Default::default() }
}

type Lines = Arc<Mutex<Vec<(LogStream, String)>>>;

fn recorder() -> (Lines, impl Fn(LogStream, &str) + Send + Sync) {
    let lines: Lines = Arc::default();
    let sink = lines.clone();
    (lines, move |s: LogStream, l: &str| sink.lock().unwrap().push((s, l.to_string())))
}

#[tokio::test(flavor = "multi_thread")]
async fn streams_lines_with_environment_and_exit_code() {
    let sup = JobObjectSupervisor::new();
    let (lines, on_line) = recorder();
    let mut spec = cmd("echo hello %LAMPO_TEST_VAR%& echo oops 1>&2& echo %PATH%& exit 3");
    spec.env = vec![("LAMPO_TEST_VAR".into(), "world".into())];
    spec.path_prepend = vec![PathBuf::from("C:\\lampo-first")];
    let code = sup.run_streaming(spec, &on_line, &CancelFlag::new()).await.unwrap();
    assert_eq!(code, Some(3));
    let lines = lines.lock().unwrap().clone();
    assert!(lines.contains(&(LogStream::Stdout, "hello world".into())), "{lines:?}");
    assert!(lines.iter().any(|(s, l)| *s == LogStream::Stderr && l.trim() == "oops"), "{lines:?}");
    assert!(lines.iter().any(|(_, l)| l.starts_with("C:\\lampo-first;")), "PATH prepended: {lines:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn carriage_returns_split_progress_lines() {
    let sup = JobObjectSupervisor::new();
    let (lines, on_line) = recorder();
    // PowerShell writes the CR-separated counters without a trailing newline in between.
    let spec = CommandSpec {
        program: "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe".into(),
        args: ["-NoProfile", "-NonInteractive", "-Command", "[Console]::Out.Write(\"10%`r50%`r100%`ndone`n\")"].iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    assert_eq!(sup.run_streaming(spec, &on_line, &CancelFlag::new()).await.unwrap(), Some(0));
    let texts: Vec<String> = lines.lock().unwrap().iter().map(|(_, l)| l.clone()).collect();
    assert_eq!(texts, ["10%", "50%", "100%", "done"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_kills_the_whole_tree() {
    let sup = Arc::new(JobObjectSupervisor::new());
    let cancel = CancelFlag::new();
    let (_, on_line) = recorder();
    let run = {
        let (sup, cancel) = (sup.clone(), cancel.clone());
        // cmd → ping: the grandchild must die too.
        tokio::spawn(async move { sup.run_streaming(cmd("ping -n 60 127.0.0.1"), &on_line, &cancel).await })
    };
    let me = std::process::id();
    let deadline = Instant::now() + Duration::from_secs(10);
    let (cmd_pid, ping_pid) = loop {
        let found = children_named(me, "cmd.exe").into_iter().find_map(|c| children_named(c, "PING.EXE").first().map(|p| (c, *p)));
        if let Some(pids) = found {
            break pids;
        }
        assert!(Instant::now() < deadline, "the process tree never appeared");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let (child, grandchild) = (ProcHandle::open(cmd_pid), ProcHandle::open(ping_pid));
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(15), run).await.expect("run returned").unwrap();
    assert_eq!(result, Err(PortError::Cancelled));
    assert!(child.exits_within(Duration::from_secs(5)), "child survived the cancel");
    assert!(grandchild.exits_within(Duration::from_secs(5)), "grandchild survived the cancel");
}

/// Real `git clone` of a tiny public repository (network). Run with
/// `cargo test -p lampo-infra-services --test streaming -- --ignored`; the destination comes from
/// `LAMPO_SCAFFOLD_SMOKE_DIR` (default: a temp folder).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs git and network"]
async fn smoke_git_clone() {
    let base = std::env::var_os("LAMPO_SCAFFOLD_SMOKE_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("lampo-scaffold-smoke"));
    std::fs::create_dir_all(&base).unwrap();
    let dest = base.join(format!("hello-{}", std::process::id()));
    let git = which_git().expect("git on PATH");
    let url = lampo_domain::scaffold::RepositoryUrl::new("https://github.com/octocat/Hello-World.git").unwrap();
    let spec = CommandSpec { program: git, args: lampo_domain::scaffold::git_clone_args(&url, None, &dest), env: lampo_domain::scaffold::git_env(), ..Default::default() };
    let sup = JobObjectSupervisor::new();
    let (lines, on_line) = recorder();
    let code = sup.run_streaming(spec, &on_line, &CancelFlag::new()).await.unwrap();
    let lines = lines.lock().unwrap().clone();
    assert_eq!(code, Some(0), "{lines:?}");
    assert!(dest.join("README").is_file(), "cloned files present");
    assert!(lines.iter().any(|(_, l)| l.starts_with("Cloning into")), "{lines:?}");
    println!("{} lines, e.g. {:?}", lines.len(), lines.iter().take(6).collect::<Vec<_>>());
}

fn which_git() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("git.exe")).find(|p| p.is_file())
}
