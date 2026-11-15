//! `CommandRunner` of the Job Object supervisor: capture, timeouts, streaming pipelines.

use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use lampo_domain::PortError;
use lampo_domain::ports::{CommandRunner, CommandSpec, PipedCommand, PipedCommandRunner};
use lampo_domain::progress::CancelFlag;
use lampo_infra_services::JobObjectSupervisor;

mod common;

fn spec(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec { program: program.into(), args: args.iter().map(|s| s.to_string()).collect(), ..Default::default() }
}

#[tokio::test(flavor = "multi_thread")]
async fn run_captures_output_env_and_working_dir() {
    let dir = tempfile::tempdir().unwrap();
    let sup = JobObjectSupervisor::new();
    let mut s = spec("cmd.exe", &["/d", "/c", "echo %LAMPO_TEST_VAR%& cd& echo err 1>&2& exit 3"]);
    s.env = vec![("LAMPO_TEST_VAR".into(), "value with spaces".into())];
    s.working_dir = Some(dir.path().to_path_buf());
    let out = sup.run(s, Duration::from_secs(10)).await.unwrap();
    assert_eq!(out.exit_code, Some(3));
    assert!(!out.success());
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(lines[0], "value with spaces");
    assert_eq!(std::fs::canonicalize(lines[1]).unwrap(), std::fs::canonicalize(dir.path()).unwrap());
    assert_eq!(out.stderr.trim(), "err");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_timeout_kills_the_command() {
    let sup = JobObjectSupervisor::new();
    let started = Instant::now();
    let err = sup.run(spec("ping.exe", &["-n", "30", "127.0.0.1"]), Duration::from_millis(500)).await.unwrap_err();
    assert!(matches!(err, PortError::Process { .. }), "{err}");
    assert!(started.elapsed() < Duration::from_secs(3));
    // The only ping started by this test binary must be gone.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(common::children_named(std::process::id(), "PING.EXE").is_empty(), "timed-out command still running");
}

fn numbered_file(dir: &std::path::Path, lines: usize) -> (std::path::PathBuf, u64) {
    let path = dir.join("input.txt");
    let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    for i in (0..lines).rev() {
        writeln!(f, "line {i:07}").unwrap();
    }
    f.flush().unwrap();
    drop(f);
    let len = std::fs::metadata(&path).unwrap().len();
    (path, len)
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_streams_file_through_two_stages_into_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let (input, len) = numbered_file(dir.path(), 200_000);
    let output = dir.path().join("out.txt");
    let sup = JobObjectSupervisor::new();
    let seen = AtomicU64::new(0);
    let calls = AtomicU64::new(0);
    let on_bytes = |b: u64| {
        seen.store(b, Ordering::SeqCst);
        calls.fetch_add(1, Ordering::SeqCst);
    };
    let command = PipedCommand { stages: vec![spec("findstr.exe", &["/r", "^"]), spec("sort.exe", &[])], stdin_file: Some(input.clone()), stdout_file: Some(output.clone()) };
    let outs = sup.run_piped(command, &on_bytes, &CancelFlag::new(), Duration::from_secs(60)).await.unwrap();
    assert_eq!(outs.len(), 2);
    assert!(outs.iter().all(|o| o.success()), "{outs:?}");
    assert_eq!(seen.load(Ordering::SeqCst), len, "bytes read from the input file");
    assert!(calls.load(Ordering::SeqCst) >= 1);
    let text = std::fs::read_to_string(&output).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 200_000);
    assert_eq!(lines[0], "line 0000000");
    assert_eq!(lines[199_999], "line 0199999");
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_relays_between_stages_and_captures_the_last_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let (input, len) = numbered_file(dir.path(), 50_000);
    let sup = JobObjectSupervisor::new();
    let seen = Mutex::new(Vec::new());
    let on_bytes = |b: u64| seen.lock().unwrap().push(b);
    let command = PipedCommand { stages: vec![spec("cmd.exe", &["/d", "/c", "type", &input.display().to_string()]), spec("sort.exe", &["/r"])], stdin_file: None, stdout_file: None };
    let outs = sup.run_piped(command, &on_bytes, &CancelFlag::new(), Duration::from_secs(60)).await.unwrap();
    let lines: Vec<&str> = outs[1].stdout.lines().collect();
    assert_eq!(lines.len(), 50_000);
    assert_eq!(lines[0], "line 0049999");
    assert_eq!(*seen.lock().unwrap().last().unwrap(), len, "bytes relayed out of the first stage");
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_cancel_kills_every_stage() {
    let sup = JobObjectSupervisor::new();
    let cancel = CancelFlag::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        c2.cancel();
    });
    let command = PipedCommand { stages: vec![spec("cmd.exe", &["/d", "/c", "ping -n 30 127.0.0.1"]), spec("sort.exe", &[])], ..Default::default() };
    let started = Instant::now();
    let err = sup.run_piped(command, &|_| {}, &cancel, Duration::from_secs(60)).await.unwrap_err();
    assert_eq!(err, PortError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_reports_missing_input_file() {
    let sup = JobObjectSupervisor::new();
    let command = PipedCommand { stages: vec![spec("sort.exe", &[])], stdin_file: Some(r"C:\no\such\dump.sql".into()), stdout_file: None };
    let err = sup.run_piped(command, &|_| {}, &CancelFlag::new(), Duration::from_secs(5)).await.unwrap_err();
    assert!(matches!(err, PortError::NotFound { entity: "file", .. }), "{err}");
}
