//! Job Object supervisor: output capture, exit reporting, termination, and the guarantee that
//! children (and grandchildren) die with the job owner.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lampo_domain::ports::{CommandSpec, ProcessObserver, ProcessSupervisor};
use lampo_domain::service::LogStream;
use lampo_infra_services::JobObjectSupervisor;

mod common;
use common::{ProcHandle, children_named};

#[derive(Default)]
struct Recorder {
    lines: Mutex<Vec<(LogStream, String)>>,
    exit: Mutex<Option<Option<i32>>>,
    exited: tokio::sync::Notify,
}

impl ProcessObserver for Recorder {
    fn on_output(&self, stream: LogStream, line: &str) {
        self.lines.lock().unwrap().push((stream, line.to_string()));
    }
    fn on_exit(&self, code: Option<i32>) {
        *self.exit.lock().unwrap() = Some(code);
        self.exited.notify_one();
    }
}

impl Recorder {
    async fn wait_exit(&self, within: Duration) -> Option<i32> {
        if let Some(code) = *self.exit.lock().unwrap() {
            return code;
        }
        tokio::time::timeout(within, self.exited.notified()).await.expect("child did not exit in time");
        self.exit.lock().unwrap().expect("exit recorded")
    }
}

fn cmd(script: &str) -> CommandSpec {
    CommandSpec { program: "cmd.exe".into(), args: vec!["/d".into(), "/s".into(), "/c".into(), script.into()], ..Default::default() }
}

fn ping(count: u32) -> CommandSpec {
    CommandSpec { program: "ping.exe".into(), args: vec!["-n".into(), count.to_string(), "127.0.0.1".into()], ..Default::default() }
}

#[tokio::test(flavor = "multi_thread")]
async fn captures_stdout_stderr_and_exit_code() {
    let sup = JobObjectSupervisor::new();
    let rec = Arc::new(Recorder::default());
    let child = sup.spawn(cmd("echo hello& echo oops 1>&2& exit 7"), rec.clone()).await.unwrap();
    assert!(child.pid > 0);
    assert_eq!(rec.wait_exit(Duration::from_secs(10)).await, Some(7));
    let lines = rec.lines.lock().unwrap().clone();
    assert!(lines.iter().any(|(s, l)| *s == LogStream::Stdout && l == "hello"), "{lines:?}");
    assert!(lines.iter().any(|(s, l)| *s == LogStream::Stderr && l.trim() == "oops"), "{lines:?}");
    assert!(!sup.is_alive(child.id));
}

#[tokio::test(flavor = "multi_thread")]
async fn splits_many_lines_without_losing_any() {
    let sup = JobObjectSupervisor::new();
    let rec = Arc::new(Recorder::default());
    sup.spawn(cmd("for /l %i in (1,1,2000) do @echo line %i"), rec.clone()).await.unwrap();
    assert_eq!(rec.wait_exit(Duration::from_secs(30)).await, Some(0));
    let lines = rec.lines.lock().unwrap();
    let stdout: Vec<_> = lines.iter().filter(|(s, _)| *s == LogStream::Stdout).map(|(_, l)| l.as_str()).collect();
    assert_eq!(stdout.len(), 2000);
    assert_eq!(stdout[0], "line 1");
    assert_eq!(stdout[1999], "line 2000");
}

#[tokio::test(flavor = "multi_thread")]
async fn terminate_waits_the_grace_period_then_kills() {
    let sup = JobObjectSupervisor::new();
    let rec = Arc::new(Recorder::default());
    let child = sup.spawn(ping(60), rec.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(sup.is_alive(child.id));
    let started = Instant::now();
    sup.terminate(child.id, Duration::from_millis(300)).await.unwrap();
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(rec.wait_exit(Duration::from_secs(1)).await, None, "killed children report no exit code");
    assert!(!sup.is_alive(child.id));
    // Terminating an exited/unknown child is fine.
    sup.terminate(child.id, Duration::from_millis(10)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_reports_missing_programs() {
    let sup = JobObjectSupervisor::new();
    let spec = CommandSpec { program: r"C:\no\such\program.exe".into(), ..Default::default() };
    let err = sup.spawn(spec, Arc::new(Recorder::default())).await.unwrap_err();
    assert!(err.to_string().contains("program.exe"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_supervisor_kills_its_children() {
    let sup = JobObjectSupervisor::new();
    let child = sup.spawn(ping(60), Arc::new(Recorder::default())).await.unwrap();
    let handle = ProcHandle::open(child.pid);
    assert!(!handle.exits_within(Duration::from_millis(100)));
    drop(sup);
    assert!(handle.exits_within(Duration::from_secs(5)), "child survived the job handle");
}

/// Not a real test: the body of the "app" process used by `children_die_when_the_app_dies`.
#[test]
#[ignore = "helper process for children_die_when_the_app_dies"]
fn job_owner_helper() {
    if std::env::var_os("LAMPO_JOB_HELPER").is_none() {
        return;
    }
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let sup = JobObjectSupervisor::new();
    // cmd → ping: a child with a grandchild.
    let child = rt.block_on(sup.spawn(cmd("ping -n 120 127.0.0.1 >NUL"), Arc::new(Recorder::default()))).unwrap();
    println!("CHILD {}", child.pid);
    std::thread::sleep(Duration::from_secs(120));
}

#[test]
fn children_die_when_the_app_dies() {
    let mut helper = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "job_owner_helper", "--ignored", "--nocapture", "--test-threads=1"])
        .env("LAMPO_JOB_HELPER", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    struct KillOnDrop(std::process::Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let stdout = helper.stdout.take().unwrap();
    let mut helper = KillOnDrop(helper);
    let child_pid: u32 = BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        // libtest prints `test job_owner_helper ... ` without a newline before the test output.
        .find_map(|l| l.split_once("CHILD ").and_then(|(_, p)| p.trim().parse().ok()))
        .expect("helper printed its child pid");
    let deadline = Instant::now() + Duration::from_secs(10);
    let grandchild = loop {
        if let Some(pid) = children_named(child_pid, "PING.EXE").first() {
            break *pid;
        }
        assert!(Instant::now() < deadline, "grandchild never appeared");
        std::thread::sleep(Duration::from_millis(50));
    };
    let child = ProcHandle::open(child_pid);
    let grandchild = ProcHandle::open(grandchild);
    assert!(!child.exits_within(Duration::from_millis(100)));

    // Simulate a crash of the app: hard kill, no cleanup code runs.
    helper.0.kill().unwrap();
    assert!(child.exits_within(Duration::from_secs(5)), "child survived the app");
    assert!(grandchild.exits_within(Duration::from_secs(5)), "grandchild survived the app");
}
