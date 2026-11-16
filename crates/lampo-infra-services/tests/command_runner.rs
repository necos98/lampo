//! `CommandRunner` of the Job Object supervisor: capture, timeouts, streaming pipelines.

use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use lampo_domain::PortError;
use lampo_domain::ports::{CommandRunner, CommandSpec, PipedCommand, PipedCommandRunner, StreamCompression};
use lampo_domain::progress::CancelFlag;
use lampo_domain::sql_dump::rewrite::DumpRewrite;
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
    let command =
        PipedCommand { stages: vec![spec("findstr.exe", &["/r", "^"]), spec("sort.exe", &[])], stdin_file: Some(input.clone()), stdout_file: Some(output.clone()), ..Default::default() };
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
    let command = PipedCommand { stages: vec![spec("cmd.exe", &["/d", "/c", "type", &input.display().to_string()]), spec("sort.exe", &["/r"])], ..Default::default() };
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
    let command = PipedCommand { stages: vec![spec("sort.exe", &[])], stdin_file: Some(r"C:\no\such\dump.sql".into()), ..Default::default() };
    let err = sup.run_piped(command, &|_| {}, &CancelFlag::new(), Duration::from_secs(5)).await.unwrap_err();
    assert!(matches!(err, PortError::NotFound { entity: "file", .. }), "{err}");
}

fn gzip_file(plain: &std::path::Path, name: &str) -> (std::path::PathBuf, u64) {
    let path = plain.with_file_name(name);
    let mut e = flate2::write::GzEncoder::new(std::fs::File::create(&path).unwrap(), flate2::Compression::fast());
    std::io::copy(&mut std::fs::File::open(plain).unwrap(), &mut e).unwrap();
    e.finish().unwrap();
    let len = std::fs::metadata(&path).unwrap().len();
    (path, len)
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_streams_a_gzip_input_and_reports_compressed_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let (plain, _) = numbered_file(dir.path(), 200_000);
    let (gz, gz_len) = gzip_file(&plain, "input.sql.gz");
    let output = dir.path().join("out.txt");
    let sup = JobObjectSupervisor::new();
    let seen = Mutex::new(Vec::new());
    let on_bytes = |b: u64| seen.lock().unwrap().push(b);
    let command = PipedCommand {
        stages: vec![spec("findstr.exe", &["/r", "^"])],
        stdin_file: Some(gz),
        stdin_compression: StreamCompression::Gzip,
        stdout_file: Some(output.clone()),
        ..Default::default()
    };
    let outs = sup.run_piped(command, &on_bytes, &CancelFlag::new(), Duration::from_secs(60)).await.unwrap();
    assert!(outs[0].success(), "{outs:?}");
    assert_eq!(std::fs::read(&output).unwrap(), std::fs::read(&plain).unwrap());
    assert_eq!(*seen.lock().unwrap().last().unwrap(), gz_len, "progress counts the compressed file");
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_streams_the_sql_entry_of_a_zip() {
    let dir = tempfile::tempdir().unwrap();
    let zip_path = dir.path().join("dump.zip");
    let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
    w.start_file("notes.txt", zip::write::SimpleFileOptions::default()).unwrap();
    w.write_all(b"ignored").unwrap();
    w.start_file("dump/shop.sql", zip::write::SimpleFileOptions::default()).unwrap();
    w.write_all(b"b\r\nc\r\na\r\n").unwrap();
    w.finish().unwrap();
    let sup = JobObjectSupervisor::new();
    let command = PipedCommand { stages: vec![spec("sort.exe", &[])], stdin_file: Some(zip_path), stdin_compression: StreamCompression::Zip, ..Default::default() };
    let outs = sup.run_piped(command, &|_| {}, &CancelFlag::new(), Duration::from_secs(30)).await.unwrap();
    assert_eq!(outs[0].stdout.lines().collect::<Vec<_>>(), ["a", "b", "c"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_fails_on_a_corrupt_gzip_input() {
    let dir = tempfile::tempdir().unwrap();
    let (plain, _) = numbered_file(dir.path(), 100_000);
    let (gz, len) = gzip_file(&plain, "cut.sql.gz");
    let bytes = std::fs::read(&gz).unwrap();
    std::fs::write(&gz, &bytes[..len as usize / 2]).unwrap();
    let sup = JobObjectSupervisor::new();
    let command = PipedCommand { stages: vec![spec("sort.exe", &[])], stdin_file: Some(gz), stdin_compression: StreamCompression::Gzip, ..Default::default() };
    let err = sup.run_piped(command, &|_| {}, &CancelFlag::new(), Duration::from_secs(30)).await.unwrap_err();
    assert!(matches!(&err, PortError::InvalidData { message, .. } if message.contains("corrupt or truncated gzip")), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_cancel_stops_a_compressed_feed() {
    let dir = tempfile::tempdir().unwrap();
    let (plain, _) = numbered_file(dir.path(), 300_000);
    let (gz, _) = gzip_file(&plain, "big.sql.gz");
    let sup = JobObjectSupervisor::new();
    let cancel = CancelFlag::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        c2.cancel();
    });
    // ping never reads its stdin: the feed blocks on a full pipe until the cancel (run via cmd,
    // like the test above: `run_timeout_kills_the_command` counts direct ping children).
    let command =
        PipedCommand { stages: vec![spec("cmd.exe", &["/d", "/c", "ping -n 30 127.0.0.1"])], stdin_file: Some(gz), stdin_compression: StreamCompression::Gzip, ..Default::default() };
    let started = Instant::now();
    let err = sup.run_piped(command, &|_| {}, &cancel, Duration::from_secs(60)).await.unwrap_err();
    assert_eq!(err, PortError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_gzips_the_last_stdout_into_the_output_file() {
    let dir = tempfile::tempdir().unwrap();
    let (plain, len) = numbered_file(dir.path(), 100_000);
    let output = dir.path().join("dump.sql.gz");
    let sup = JobObjectSupervisor::new();
    let seen = Mutex::new(Vec::new());
    let on_bytes = |b: u64| seen.lock().unwrap().push(b);
    let command = PipedCommand {
        stages: vec![spec("cmd.exe", &["/d", "/c", "type", &plain.display().to_string()])],
        stdout_file: Some(output.clone()),
        stdout_compression: StreamCompression::Gzip,
        ..Default::default()
    };
    let outs = sup.run_piped(command, &on_bytes, &CancelFlag::new(), Duration::from_secs(60)).await.unwrap();
    assert!(outs[0].success(), "{outs:?}");
    let mut decoded = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(std::fs::File::open(&output).unwrap()), &mut decoded).unwrap();
    assert_eq!(decoded, std::fs::read(&plain).unwrap());
    assert!(std::fs::metadata(&output).unwrap().len() < len / 2, "compressed");
    assert_eq!(*seen.lock().unwrap().last().unwrap(), len, "progress counts the uncompressed bytes");
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_rewrites_a_plain_input_on_its_way_without_changing_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("dump.sql");
    let dump = "/*M!999999\\- enable the sandbox mode */\r\nCREATE DEFINER=`app`@`localhost` PROCEDURE p()\r\nINSERT INTO t VALUES ('CREATE DEFINER=`x`@`y`');\r\n";
    std::fs::write(&input, dump).unwrap();
    let output = dir.path().join("out.txt");
    let sup = JobObjectSupervisor::new();
    let seen = AtomicU64::new(0);
    let rewrite = DumpRewrite { drop_sandbox_line: true, strip_definers: true };
    let command =
        PipedCommand { stages: vec![spec("findstr.exe", &["/r", "^"])], stdin_file: Some(input.clone()), stdout_file: Some(output.clone()), stdin_rewrite: rewrite, ..Default::default() };
    let outs = sup.run_piped(command, &|b| seen.store(b, Ordering::SeqCst), &CancelFlag::new(), Duration::from_secs(30)).await.unwrap();
    assert!(outs.iter().all(|o| o.success()), "{outs:?}");
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "CREATE PROCEDURE p()\r\nINSERT INTO t VALUES ('CREATE DEFINER=`x`@`y`');\r\n");
    assert_eq!(std::fs::read_to_string(&input).unwrap(), dump, "the source file is untouched");
    assert_eq!(seen.load(Ordering::SeqCst), dump.len() as u64, "progress counts the file's bytes");
}
