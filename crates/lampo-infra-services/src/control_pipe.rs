//! Named-pipe transport for the control channel (`tokio::net::windows::named_pipe`).
//! Pipe name: `lampo_application::control::pipe_name(<user>)`, computed by the composition root.
//!
//! **Security.** The server creates every pipe instance with a descriptor owned by the current
//! user whose protected DACL grants access to that user only (`O:<sid>D:P(A;;GA;;;<sid>)`), and
//! with `reject_remote_clients(true)`. The first instance uses `FILE_FLAG_FIRST_PIPE_INSTANCE`,
//! so a second app (or a squatter) cannot share the name. The client refuses a pipe whose owner
//! is not the current user and connects at `SECURITY_IDENTIFICATION` level (the server cannot
//! impersonate it).
//!
//! **Framing.** Newline-delimited UTF-8 JSON. The client writes one line: the request frame,
//! verbatim. The server answers with lines `{"event":<frame>}` (zero or more, written as they
//! are produced) and one `{"final":<frame>}`; the inner frames are the application's frames,
//! verbatim. The wrapper is the transport's only knowledge and never changes; schema and
//! protocol version are checked by the application (an incompatible app replies with a
//! protocol-mismatch frame, and an unrecognised line is handed to the application as the
//! final frame so it reports the mismatch too). Lines are capped (`MAX_FRAME`).
//!
//! **Lifecycle.** Each connection is served on its own task; a new instance is listening
//! before the accepted one is handled, so concurrent clients never see "not found". The server
//! keeps reading after the request: EOF or an error means the client went away (Ctrl-C) and
//! sets the handler's `CancelFlag`. After the final frame the server waits (bounded) for the
//! client to close, so the reply is never discarded by an early disconnect.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::ports::{ControlChannelClient, ControlChannelServer, ControlFrameSink, ControlRequestHandler};
use lampo_domain::progress::CancelFlag;
use lampo_domain::{PortError, PortResult};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};
use tokio::sync::mpsc;
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_SEM_TIMEOUT};
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

use crate::win::{self, UserOnlySecurity, wide};

/// Longest accepted line (a request, or a frame such as a large status payload).
const MAX_FRAME: usize = 16 * 1024 * 1024;
/// A connected client must send its request within this time.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// After the final frame, how long the server waits for the client to close.
const LINGER: Duration = Duration::from_secs(10);
const EVENT_PREFIX: &str = "{\"event\":";
const FINAL_PREFIX: &str = "{\"final\":";

/// JSON never needs a raw line break (inside strings they are escaped), so replacing them keeps
/// the frame valid and the framing intact.
fn one_line(json: &str) -> String {
    json.replace(['\r', '\n'], " ")
}

/// Reads one `\n`-terminated line (without the terminator). `Ok(None)` on EOF.
async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let n = (&mut *reader).take(MAX_FRAME as u64 + 1).read_until(b'\n', &mut buf).await?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() != Some(&b'\n') {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, if buf.len() > MAX_FRAME { "frame too large" } else { "connection closed mid-frame" }));
    }
    buf.pop();
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    String::from_utf8(buf).map(Some).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub struct NamedPipeControlServer {
    pipe_name: String,
}

impl NamedPipeControlServer {
    pub fn new(pipe_name: String) -> Self {
        Self { pipe_name }
    }

    fn create(&self, security: &UserOnlySecurity, first: bool) -> std::io::Result<NamedPipeServer> {
        create_pipe_instance(&self.pipe_name, security, first)
    }
}

/// Creates one server instance of `pipe_name` with `security` (the only way Lampo creates one).
fn create_pipe_instance(pipe_name: &str, security: &UserOnlySecurity, first: bool) -> std::io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first).reject_remote_clients(true).in_buffer_size(64 * 1024).out_buffer_size(64 * 1024);
    // SAFETY: valid SECURITY_ATTRIBUTES alive for the call.
    unsafe { options.create_with_security_attributes_raw(pipe_name, security.as_ptr() as *mut std::ffi::c_void) }
}

/// Raw server pipes for tests that need to speak something other than Lampo's framing. Not part
/// of the public API.
#[doc(hidden)]
pub mod test_support {
    use super::*;

    /// A first-instance server pipe created exactly like [`NamedPipeControlServer`]'s (owned by
    /// the current user, user-only DACL), so the client's owner check accepts it in any context,
    /// elevated or not. (Default security would make BUILTIN\Administrators the owner when the
    /// process is elevated.)
    pub fn create_lampo_server_pipe(pipe_name: &str) -> std::io::Result<NamedPipeServer> {
        create_pipe_instance(pipe_name, &UserOnlySecurity::new(false)?, true)
    }

    /// A first-instance server pipe whose descriptor is `sddl` (e.g. `O:BA...` for one owned by
    /// Administrators). Setting an owner the caller's token cannot assign fails (`ERROR_INVALID_OWNER`).
    pub fn create_server_pipe_with_sddl(pipe_name: &str, sddl: &str) -> std::io::Result<NamedPipeServer> {
        create_pipe_instance(pipe_name, &UserOnlySecurity::from_sddl(sddl, false)?, true)
    }
}

/// Pushes intermediate frames to the connection's writer without blocking the handler.
struct ChannelSink(mpsc::UnboundedSender<String>);

impl ControlFrameSink for ChannelSink {
    fn frame(&self, json: String) {
        let _ = self.0.send(json);
    }
}

async fn serve_connection(pipe: NamedPipeServer, handler: Arc<dyn ControlRequestHandler>) {
    let (read_half, mut write_half) = tokio::io::split(pipe);
    let mut reader = BufReader::new(read_half);
    let request = match tokio::time::timeout(REQUEST_TIMEOUT, read_frame(&mut reader)).await {
        Ok(Ok(Some(line))) => line,
        // Probe, timeout or garbage: nothing to answer.
        _ => return,
    };

    let cancel = CancelFlag::new();
    let watcher_cancel = cancel.clone();
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel::<()>();
    let watcher = tokio::spawn(async move {
        let mut buf = [0u8; 256];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {} // Nothing else is expected from the client; ignore.
            }
        }
        watcher_cancel.cancel();
        let _ = closed_tx.send(());
    });

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let handling = async {
        let sink = ChannelSink(tx);
        handler.handle(request, &sink, &cancel).await
    };
    let writing = async {
        let mut writable = true;
        while let Some(frame) = rx.recv().await {
            if writable {
                let line = format!("{EVENT_PREFIX}{}}}\n", one_line(&frame));
                writable = write_half.write_all(line.as_bytes()).await.is_ok();
                if !writable {
                    cancel.cancel();
                }
            }
        }
        writable
    };
    let (final_frame, writable) = tokio::join!(handling, writing);
    if writable {
        let line = format!("{FINAL_PREFIX}{}}}\n", one_line(&final_frame));
        if write_half.write_all(line.as_bytes()).await.is_ok() && write_half.flush().await.is_ok() {
            let _ = tokio::time::timeout(LINGER, closed_rx).await;
        }
    }
    watcher.abort();
}

#[async_trait]
impl ControlChannelServer for NamedPipeControlServer {
    async fn serve(&self, handler: Arc<dyn ControlRequestHandler>) -> PortResult<()> {
        let security = UserOnlySecurity::new(false).map_err(|e| PortError::io("creating the control pipe security descriptor", e))?;
        let mut listening = self.create(&security, true).map_err(|e| {
            if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
                PortError::AlreadyExists { entity: "control pipe", id: self.pipe_name.clone() }
            } else {
                PortError::io(format!("creating {}", self.pipe_name), e)
            }
        })?;
        loop {
            let connected = listening.connect().await;
            // Always have the next instance listening before serving this one.
            let next = self.create(&security, false).map_err(|e| PortError::io(format!("creating {}", self.pipe_name), e))?;
            let accepted = std::mem::replace(&mut listening, next);
            match connected {
                Ok(()) => {
                    tokio::spawn(serve_connection(accepted, handler.clone()));
                }
                // The client vanished between connecting and being accepted.
                Err(e) => tracing::debug!(error = %e, "control pipe connection dropped before it was accepted"),
            }
        }
    }
}

pub struct NamedPipeControlClient {
    pipe_name: String,
}

impl NamedPipeControlClient {
    pub fn new(pipe_name: String) -> Self {
        Self { pipe_name }
    }

    fn app_not_running(&self) -> PortError {
        PortError::not_found("app", self.pipe_name.clone())
    }

    /// Opens the pipe, retrying while every instance is momentarily busy.
    async fn connect(&self) -> PortResult<NamedPipeClient> {
        let mut delay = Duration::from_millis(10);
        loop {
            match ClientOptions::new().open(&self.pipe_name) {
                Ok(client) => return Ok(client),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_millis(200));
                }
                Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) => return Err(self.app_not_running()),
                Err(e) => return Err(PortError::io(format!("connecting to {}", self.pipe_name), e)),
            }
        }
    }

    async fn exchange_inner(&self, request_json: String, events: &dyn ControlFrameSink) -> PortResult<String> {
        let client = self.connect().await?;
        let owned_by_me = {
            use std::os::windows::io::AsRawHandle;
            win::is_owned_by_current_user(client.as_raw_handle() as _)
        };
        match owned_by_me {
            Ok(true) => {}
            Ok(false) => return Err(PortError::io(format!("connecting to {}", self.pipe_name), "the pipe is not owned by the current user")),
            Err(e) => return Err(PortError::io(format!("checking the owner of {}", self.pipe_name), e)),
        }
        let (read_half, mut write_half) = tokio::io::split(client);
        let io_err = |e: std::io::Error| PortError::io("talking to the Lampo app", e);
        write_half.write_all(format!("{}\n", one_line(&request_json)).as_bytes()).await.map_err(io_err)?;
        write_half.flush().await.map_err(io_err)?;
        let mut reader = BufReader::new(read_half);
        loop {
            let Some(line) = read_frame(&mut reader).await.map_err(io_err)? else {
                return Err(PortError::io("talking to the Lampo app", "the app closed the connection without replying"));
            };
            if let Some(inner) = line.strip_prefix(EVENT_PREFIX).and_then(|l| l.strip_suffix('}')) {
                events.frame(inner.to_string());
            } else if let Some(inner) = line.strip_prefix(FINAL_PREFIX).and_then(|l| l.strip_suffix('}')) {
                return Ok(inner.to_string());
            } else {
                // Not our framing: an incompatible app. Let the application decode (and report) it.
                return Ok(line);
            }
        }
    }
}

#[async_trait]
impl ControlChannelClient for NamedPipeControlClient {
    fn is_server_running(&self) -> bool {
        let name = wide(&self.pipe_name);
        // SAFETY: valid NUL-terminated name. Does not connect (no instance is consumed).
        if unsafe { WaitNamedPipeW(name.as_ptr(), 1) } != 0 {
            return true;
        }
        // Every instance busy (timeout) still means an app is listening.
        let code = std::io::Error::last_os_error().raw_os_error();
        code == Some(ERROR_SEM_TIMEOUT as i32) || code == Some(ERROR_PIPE_BUSY as i32)
    }

    async fn exchange(&self, request_json: String, events: &dyn ControlFrameSink, timeout: Duration) -> PortResult<String> {
        match tokio::time::timeout(timeout, self.exchange_inner(request_json, events)).await {
            Ok(result) => result,
            Err(_) => Err(PortError::io("waiting for the Lampo app", format!("no reply within {}s", timeout.as_secs()))),
        }
    }
}
