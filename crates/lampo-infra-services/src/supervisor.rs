//! One Windows Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) for every child: if the app
//! dies, the OS kills all services. Children are created inside the job with `CREATE_NO_WINDOW`
//! (see [`crate::process`]); stdout/stderr are read line by line on the I/O completion port and
//! forwarded to the observer (runners append them to the service's log file).
//!
//! Exit detection uses a system thread-pool wait per child; nothing polls. Dropping the
//! supervisor closes the job handle, which kills every child.
//!
//! Graceful stop: console programs started without a shared console have no generic stop
//! signal, so each runner first sends its program's own shutdown request (MySQL shutdown event /
//! `mysqladmin shutdown`, Redis `SHUTDOWN`) and then calls [`ProcessSupervisor::terminate`],
//! which waits up to `grace` for the exit before killing.

use std::collections::HashMap;
use std::os::windows::io::OwnedHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::ports::{
    ChildId, CommandOutput, CommandRunner, CommandSpec, PipedCommand, PipedCommandRunner, ProcessObserver, ProcessSupervisor, SpawnedChild, StreamCompression, StreamingCommandRunner,
};
use lampo_domain::progress::CancelFlag;
use lampo_domain::service::LogStream;
use lampo_domain::{PortError, PortResult};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::{mpsc, watch};
use windows_sys::Win32::Storage::FileSystem::{FILE_CURRENT, GetFileSizeEx, SetFilePointerEx};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::TerminateProcess;

use crate::compression;
use crate::process::{self, Input, Output};
use crate::win::{self, owned, raw};

/// Longest line forwarded to an observer; longer lines are split.
const MAX_LINE: usize = 16 * 1024;
/// Captured stdout of a command run to completion (`run`, last stage of `run_piped`).
const MAX_CAPTURED_STDOUT: usize = 16 * 1024 * 1024;
const MAX_CAPTURED_STDERR: usize = 1024 * 1024;
/// After a child exits, how long its output may still be drained before `on_exit`.
const DRAIN_AFTER_EXIT: Duration = Duration::from_secs(2);
/// After a kill, how long to wait for the OS to report the exit.
const KILL_WAIT: Duration = Duration::from_secs(10);
const PROGRESS_TICK: Duration = Duration::from_millis(250);

struct ChildEntry {
    pid: u32,
    handle: Arc<OwnedHandle>,
    killed: AtomicBool,
    /// Becomes `true` after `on_exit` has been delivered.
    done: watch::Receiver<bool>,
}

type ChildTable = Arc<Mutex<HashMap<ChildId, Arc<ChildEntry>>>>;

pub struct JobObjectSupervisor {
    job: OnceLock<Result<OwnedHandle, String>>,
    next_id: AtomicU64,
    children: ChildTable,
}

impl JobObjectSupervisor {
    /// Does no IO; the Job Object is created on first use.
    pub fn new() -> Self {
        Self { job: OnceLock::new(), next_id: AtomicU64::new(1), children: Arc::default() }
    }

    fn job(&self) -> PortResult<&OwnedHandle> {
        self.job.get_or_init(|| create_job().map_err(|e| e.to_string())).as_ref().map_err(|e| PortError::process("job object", e))
    }

    fn entry(&self, child: ChildId) -> Option<Arc<ChildEntry>> {
        self.children.lock().ok()?.get(&child).cloned()
    }
}

impl Default for JobObjectSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

fn create_job() -> std::io::Result<OwnedHandle> {
    // SAFETY: anonymous job, default security.
    let job = owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })?;
    // SAFETY: plain C struct.
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: valid job handle and correctly sized struct.
    let ok = unsafe { SetInformationJobObject(raw(&job), JobObjectExtendedLimitInformation, (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(), size_of_val(&info) as u32) };
    if ok == 0 { Err(std::io::Error::last_os_error()) } else { Ok(job) }
}

fn program_name(program: &Path) -> String {
    program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| program.display().to_string())
}

fn kill(handle: &OwnedHandle) {
    // SAFETY: valid process handle; failure means it already exited.
    unsafe { TerminateProcess(raw(handle), 1) };
}

/// Waits for the process to exit (thread-pool wait; slow polling only if registration fails).
async fn exited(handle: &OwnedHandle) {
    if win::wait_for_exit(handle).await.is_err() {
        while !win::has_exited(handle) {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

fn decode_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.trim_end_matches(['\r', '\n']).to_string()
}

/// Forwards complete lines (split at `MAX_LINE`) until EOF.
async fn forward_lines(pipe: NamedPipeServer, stream: LogStream, observer: Arc<dyn ProcessObserver>) {
    let mut reader = BufReader::with_capacity(8 * 1024, pipe);
    let mut line = Vec::new();
    loop {
        let buf = match reader.fill_buf().await {
            Ok([]) | Err(_) => break,
            Ok(buf) => buf,
        };
        let room = MAX_LINE - line.len();
        let (take, complete) = match buf.iter().take(room).position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (buf.len().min(room), line.len() + buf.len().min(room) >= MAX_LINE),
        };
        line.extend_from_slice(&buf[..take]);
        reader.consume(take);
        if complete {
            observer.on_output(stream, &decode_line(&line));
            line.clear();
        }
    }
    if !line.is_empty() {
        observer.on_output(stream, &decode_line(&line));
    }
}

/// Reads to EOF, keeping at most `cap` bytes (the rest is drained so the child never blocks).
async fn read_capped(pipe: NamedPipeServer, cap: usize) -> Vec<u8> {
    let mut reader = pipe;
    let mut out = Vec::new();
    let mut buf = vec![0u8; 16 * 1024];
    while let Ok(n) = reader.read(&mut buf).await {
        if n == 0 {
            break;
        }
        let keep = n.min(cap.saturating_sub(out.len()));
        out.extend_from_slice(&buf[..keep]);
    }
    out
}

/// Copies one stage's stdout into the next stage's stdin. Closing `to` gives the reader EOF;
/// dropping `from` on a write error makes the writer fail fast (broken pipe).
async fn relay(mut from: NamedPipeServer, mut to: NamedPipeServer, counter: Option<Arc<AtomicU64>>) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
        if let Some(c) = &counter {
            c.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
    let _ = to.flush().await;
}

fn open_input(path: &Path) -> PortResult<std::fs::File> {
    std::fs::File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { PortError::not_found("file", path.display().to_string()) } else { PortError::io(format!("opening {}", path.display()), e) }
    })
}

/// Why feeding a compressed input failed, with the stage's stdin still open (see [`feed`]).
type FeedFailure = (String, NamedPipeServer);

/// Decompresses `path` (on a blocking thread, a few chunks in flight) into the first stage's
/// stdin; `position` = bytes of the compressed file consumed. A stage that stops reading ends it
/// quietly (its own failure is reported by its exit code). On corrupt data the pipe is returned
/// still open, so the caller can kill the stages before they see EOF.
async fn feed(path: PathBuf, compression: StreamCompression, position: Arc<AtomicU64>, mut stdin: NamedPipeServer) -> Result<(), FeedFailure> {
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(4);
    let decoder = tokio::task::spawn_blocking(move || compression::decode(&path, compression, &position, |chunk| tx.blocking_send(chunk).is_ok()));
    let mut consumer_gone = false;
    while let Some(chunk) = rx.recv().await {
        if stdin.write_all(&chunk).await.is_err() {
            consumer_gone = true;
            break;
        }
    }
    drop(rx);
    match decoder.await {
        Ok(Err(message)) if !consumer_gone => Err((message, stdin)),
        Err(e) if !consumer_gone => Err((e.to_string(), stdin)),
        _ => {
            let _ = stdin.flush().await;
            Ok(())
        }
    }
}

/// Gzips the last stage's stdout into `file` (on a blocking thread); `counter` = uncompressed
/// bytes. A write failure drops the pipe, so the stage fails fast with a broken pipe.
async fn encode_gzip(mut from: NamedPipeServer, file: std::fs::File, counter: Arc<AtomicU64>) -> std::io::Result<()> {
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(4);
    let writer = tokio::task::spawn_blocking(move || compression::encode_gzip(file, || rx.blocking_recv()));
    let mut buf = vec![0u8; compression::CHUNK];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        counter.fetch_add(n as u64, Ordering::Relaxed);
        if tx.send(buf[..n].to_vec()).await.is_err() {
            break;
        }
    }
    drop(tx);
    drop(from);
    writer.await.map_err(std::io::Error::other)?
}

fn file_position(h: &OwnedHandle) -> u64 {
    let mut pos = 0i64;
    // SAFETY: valid file handle; zero move just reads the shared position.
    unsafe { SetFilePointerEx(raw(h), 0, &mut pos, FILE_CURRENT) };
    pos.max(0) as u64
}

fn file_size(h: &OwnedHandle) -> u64 {
    let mut size = 0i64;
    // SAFETY: valid file handle.
    unsafe { GetFileSizeEx(raw(h), &mut size) };
    size.max(0) as u64
}

/// Kills every process not yet known to have exited (cancel, timeout, dropped future).
struct KillOnDrop(Vec<Arc<OwnedHandle>>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        for h in &self.0 {
            if !win::has_exited(h) {
                kill(h);
            }
        }
    }
}

#[async_trait]
impl ProcessSupervisor for JobObjectSupervisor {
    async fn spawn(&self, spec: CommandSpec, observer: Arc<dyn ProcessObserver>) -> PortResult<SpawnedChild> {
        let job = self.job()?;
        let p = process::spawn(&spec, &[job], Input::Null, Output::Pipe).map_err(|e| PortError::process(program_name(&spec.program), e))?;
        let id = ChildId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let (done_tx, done_rx) = watch::channel(false);
        let entry = Arc::new(ChildEntry { pid: p.pid, handle: p.handle.clone(), killed: AtomicBool::new(false), done: done_rx });
        if let Ok(mut table) = self.children.lock() {
            table.insert(id, entry.clone());
        }

        let mut readers = Vec::new();
        if let Some(stdout) = p.stdout {
            readers.push(tokio::spawn(forward_lines(stdout, LogStream::Stdout, observer.clone())));
        }
        readers.push(tokio::spawn(forward_lines(p.stderr, LogStream::Stderr, observer.clone())));

        let children = self.children.clone();
        tokio::spawn(async move {
            exited(&entry.handle).await;
            let aborts: Vec<_> = readers.iter().map(|r| r.abort_handle()).collect();
            let drain = async {
                for r in readers {
                    let _ = r.await;
                }
            };
            if tokio::time::timeout(DRAIN_AFTER_EXIT, drain).await.is_err() {
                // A grandchild still holds the pipe: stop forwarding, the child is gone.
                aborts.iter().for_each(|a| a.abort());
            }
            let code = if entry.killed.load(Ordering::SeqCst) { None } else { win::exit_code(&entry.handle) };
            observer.on_exit(code);
            if let Ok(mut table) = children.lock() {
                table.remove(&id);
            }
            let _ = done_tx.send(true);
        });
        Ok(SpawnedChild { id, pid: p.pid })
    }

    async fn terminate(&self, child: ChildId, grace: Duration) -> PortResult<()> {
        let Some(entry) = self.entry(child) else { return Ok(()) };
        let mut done = entry.done.clone();
        if !grace.is_zero() && tokio::time::timeout(grace, done.wait_for(|d| *d)).await.is_ok() {
            return Ok(());
        }
        if !win::has_exited(&entry.handle) {
            entry.killed.store(true, Ordering::SeqCst);
            kill(&entry.handle);
        }
        match tokio::time::timeout(KILL_WAIT, done.wait_for(|d| *d)).await {
            Ok(_) => Ok(()),
            Err(_) => Err(PortError::process(format!("pid {}", entry.pid), "did not exit after being killed")),
        }
    }

    fn is_alive(&self, child: ChildId) -> bool {
        self.entry(child).is_some_and(|e| !win::has_exited(&e.handle))
    }
}

#[async_trait]
impl CommandRunner for JobObjectSupervisor {
    /// Dropping the returned future kills the command.
    async fn run(&self, spec: CommandSpec, timeout: Duration) -> PortResult<CommandOutput> {
        let command = PipedCommand { stages: vec![spec], ..Default::default() };
        let mut outputs = self.run_piped(command, &|_| {}, &CancelFlag::new(), timeout).await?;
        outputs.pop().ok_or_else(|| PortError::process("command", "no output"))
    }
}

#[async_trait]
impl PipedCommandRunner for JobObjectSupervisor {
    /// Stages run inside the job; file endpoints are inherited directly by the children (no
    /// copy through this process), stage-to-stage data is relayed through IOCP pipes.
    /// Dropping the returned future kills every stage.
    async fn run_piped(&self, command: PipedCommand, on_bytes: &(dyn Fn(u64) + Send + Sync), cancel: &CancelFlag, timeout: Duration) -> PortResult<Vec<CommandOutput>> {
        let Some(first) = command.stages.first() else {
            return Err(PortError::invalid_data("piped command", "no stages"));
        };
        let program = program_name(&first.program);
        if command.stdout_compression == StreamCompression::Zip {
            return Err(PortError::Unsupported("zip-compressed command output".into()));
        }
        let job = self.job()?;
        // A compressed input is decoded here and written into a pipe; a plain one is inherited.
        let feed_path = command.stdin_file.clone().filter(|_| command.stdin_compression != StreamCompression::None);
        let stdin_file: Option<OwnedHandle> = match &command.stdin_file {
            Some(path) => {
                let file = open_input(path)?;
                if feed_path.is_some() { None } else { Some(file.into()) }
            }
            None => None,
        };
        let out_file = match &command.stdout_file {
            Some(path) => Some(std::fs::File::create(path).map_err(|e| PortError::io(format!("creating {}", path.display()), e))?),
            None => None,
        };
        let (stdout_file, encode_into): (Option<OwnedHandle>, _) = if command.stdout_compression == StreamCompression::None { (out_file.map(Into::into), None) } else { (None, out_file) };

        let n = command.stages.len();
        let mut guard = KillOnDrop(Vec::with_capacity(n));
        let mut procs = Vec::with_capacity(n);
        for (i, spec) in command.stages.iter().enumerate() {
            let input = match (i, &stdin_file) {
                (0, Some(f)) => Input::File(f),
                (0, None) if feed_path.is_none() => Input::Null,
                _ => Input::Pipe,
            };
            let output = match (i + 1 == n, &stdout_file) {
                (true, Some(f)) => Output::File(f),
                _ => Output::Pipe,
            };
            let p = process::spawn(spec, &[job], input, output).map_err(|e| PortError::process(program_name(&spec.program), e))?;
            guard.0.push(p.handle.clone());
            procs.push(p);
        }

        let relayed = Arc::new(AtomicU64::new(0));
        for i in 0..n.saturating_sub(1) {
            let from = procs[i].stdout.take();
            let to = procs[i + 1].stdin.take();
            if let (Some(from), Some(to)) = (from, to) {
                tokio::spawn(relay(from, to, (i == 0).then(|| relayed.clone())));
            }
        }
        let fed = Arc::new(AtomicU64::new(0));
        let feeder = match (&feed_path, procs[0].stdin.take()) {
            (Some(path), Some(stdin)) => Some(tokio::spawn(feed(path.clone(), command.stdin_compression, fed.clone(), stdin))),
            _ => None,
        };
        let encoded = Arc::new(AtomicU64::new(0));
        let encoder = match (encode_into, procs[n - 1].stdout.take()) {
            (Some(file), Some(stdout)) => Some(tokio::spawn(encode_gzip(stdout, file, encoded.clone()))),
            (_, stdout) => {
                procs[n - 1].stdout = stdout;
                None
            }
        };
        let stdout_capture = procs[n - 1].stdout.take().map(|p| tokio::spawn(read_capped(p, MAX_CAPTURED_STDOUT)));
        let mut handles = Vec::with_capacity(n);
        let mut stderr_captures = Vec::with_capacity(n);
        for p in procs {
            handles.push(p.handle);
            stderr_captures.push(tokio::spawn(read_capped(p.stderr, MAX_CAPTURED_STDERR)));
        }

        let has_feeder = feeder.is_some();
        let measure = || match (&stdin_file, &stdout_file) {
            _ if has_feeder => fed.load(Ordering::Relaxed),
            (Some(f), _) => file_position(f),
            (None, _) if n > 1 => relayed.load(Ordering::Relaxed),
            (None, Some(f)) => file_size(f),
            (None, None) => encoded.load(Ordering::Relaxed),
        };
        let input_name = feed_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        let feeding = async {
            match feeder {
                Some(task) => task.await.unwrap_or(Ok(())),
                None => std::future::pending().await,
            }
        };
        tokio::pin!(feeding);
        let mut feeding_done = !has_feeder;

        let finished = async {
            for h in &handles {
                exited(h).await;
            }
            let mut stderrs = Vec::with_capacity(n);
            for c in stderr_captures {
                stderrs.push(tokio::time::timeout(DRAIN_AFTER_EXIT, c).await.ok().and_then(Result::ok).unwrap_or_default());
            }
            let stdout = match stdout_capture {
                Some(c) => tokio::time::timeout(DRAIN_AFTER_EXIT, c).await.ok().and_then(Result::ok).unwrap_or_default(),
                None => Vec::new(),
            };
            (stderrs, stdout)
        };
        tokio::pin!(finished);
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        let mut tick = tokio::time::interval(PROGRESS_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last = None;
        let (stderrs, stdout) = loop {
            tokio::select! {
                out = &mut finished => break out,
                result = &mut feeding, if !feeding_done => {
                    feeding_done = true;
                    if let Err((message, stdin)) = result {
                        // Kill before the pipe closes: the stages never see a truncated input end.
                        drop(guard);
                        drop(stdin);
                        return Err(PortError::invalid_data(input_name, message));
                    }
                }
                _ = tick.tick() => {
                    if cancel.is_cancelled() {
                        return Err(PortError::Cancelled);
                    }
                    let bytes = measure();
                    if last != Some(bytes) {
                        on_bytes(bytes);
                        last = Some(bytes);
                    }
                }
                _ = &mut deadline => return Err(PortError::process(program, format!("timed out after {}s", timeout.as_secs()))),
            }
        };
        if !feeding_done && let Ok(Err((message, _))) = tokio::time::timeout(DRAIN_AFTER_EXIT, &mut feeding).await {
            return Err(PortError::invalid_data(input_name, message));
        }
        if let Some(task) = encoder {
            let path = command.stdout_file.as_deref().map(Path::display).map(|p| p.to_string()).unwrap_or_default();
            task.await.map_err(std::io::Error::other).and_then(|r| r).map_err(|e| PortError::io(format!("writing {path}"), e))?;
        }
        let bytes = measure();
        if last != Some(bytes) {
            on_bytes(bytes);
        }
        drop(guard);

        let last_index = n - 1;
        Ok(handles
            .iter()
            .zip(stderrs)
            .enumerate()
            .map(|(i, (h, stderr))| CommandOutput {
                exit_code: win::exit_code(h),
                stdout: if i == last_index { String::from_utf8_lossy(&stdout).into_owned() } else { String::new() },
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------------------------
// Streaming commands: one nested job per run, so a cancel kills the whole process tree
// ---------------------------------------------------------------------------------------------

/// The job of one streaming run (nested in the app's job). Terminating it kills every process
/// the command started; dropping it does too.
struct TreeJob(OwnedHandle);

impl TreeJob {
    fn kill(&self) {
        // SAFETY: valid job handle; failure means the job is already empty.
        unsafe { TerminateJobObject(raw(&self.0), 1) };
    }
}

impl Drop for TreeJob {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Reads to EOF and sends every non-empty line, split at `\n` *and* `\r` (progress counters
/// such as git's `Receiving objects: 45%` are rewritten with `\r`) and at `MAX_LINE`.
async fn send_lines(pipe: NamedPipeServer, stream: LogStream, tx: mpsc::UnboundedSender<(LogStream, String)>) {
    let mut reader = pipe;
    let mut buf = vec![0u8; 8 * 1024];
    let mut line: Vec<u8> = Vec::new();
    let flush = |line: &mut Vec<u8>| {
        if !line.is_empty() {
            let _ = tx.send((stream, String::from_utf8_lossy(line).into_owned()));
            line.clear();
        }
    };
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for &b in &buf[..n] {
            if b == b'\n' || b == b'\r' {
                flush(&mut line);
            } else {
                line.push(b);
                if line.len() >= MAX_LINE {
                    flush(&mut line);
                }
            }
        }
    }
    flush(&mut line);
}

#[async_trait]
impl StreamingCommandRunner for JobObjectSupervisor {
    /// Dropping the returned future kills the process tree too.
    async fn run_streaming(&self, spec: CommandSpec, on_line: &(dyn for<'l> Fn(LogStream, &'l str) + Send + Sync), cancel: &CancelFlag) -> PortResult<Option<i32>> {
        let program = program_name(&spec.program);
        cancel.check()?;
        let outer = self.job()?;
        let tree = TreeJob(create_job().map_err(|e| PortError::process("job object", e))?);
        let p = process::spawn(&spec, &[outer, &tree.0], Input::Null, Output::Pipe).map_err(|e| PortError::process(&program, e))?;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut readers = Vec::new();
        if let Some(stdout) = p.stdout {
            readers.push(tokio::spawn(send_lines(stdout, LogStream::Stdout, tx.clone())));
        }
        readers.push(tokio::spawn(send_lines(p.stderr, LogStream::Stderr, tx)));
        let abort_readers = |readers: &[tokio::task::JoinHandle<()>]| readers.iter().for_each(|r| r.abort());

        let exit = exited(&p.handle);
        tokio::pin!(exit);
        let mut tick = tokio::time::interval(PROGRESS_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                Some((stream, line)) = rx.recv() => on_line(stream, &line),
                _ = &mut exit => break,
                _ = tick.tick() => {
                    if cancel.is_cancelled() {
                        tree.kill();
                        abort_readers(&readers);
                        // Wait for the exit so the caller can clean up files the tree had open.
                        let _ = tokio::time::timeout(KILL_WAIT, &mut exit).await;
                        return Err(PortError::Cancelled);
                    }
                }
            }
        }
        // Lines still in the pipes; a leftover grandchild holding them is cut off.
        let drain = async {
            while let Some((stream, line)) = rx.recv().await {
                on_line(stream, &line);
            }
        };
        if tokio::time::timeout(DRAIN_AFTER_EXIT, drain).await.is_err() {
            abort_readers(&readers);
        }
        let code = win::exit_code(&p.handle);
        // Whatever the command left running in its tree.
        drop(tree);
        Ok(code)
    }
}
