//! Process creation shared by the supervisor (and nothing else): `CreateProcessW` with
//!
//! * `PROC_THREAD_ATTRIBUTE_JOB_LIST`: the child is created *inside* the Job Object, so there
//!   is no window in which it (or anything it spawns) could escape the job;
//! * `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`: the child inherits exactly its three stdio handles,
//!   never another child's pipe ends (which would delay EOF detection);
//! * `CREATE_NO_WINDOW`: console programs get a hidden console, nothing flashes.
//!
//! Stdio pipes are named pipes whose server end is opened overlapped and driven by tokio
//! (I/O completion port): reading a child's output costs no thread. (tokio's own
//! `process::Command` reads child stdio on the blocking thread pool on Windows.)

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{IntoRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use lampo_domain::ports::CommandSpec;
use tokio::net::windows::named_pipe::NamedPipeServer;
use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND,
};
use windows_sys::Win32::System::Pipes::{CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, InitializeProcThreadAttributeList,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute,
};

use crate::win::{self, UserOnlySecurity, owned, raw, wide};

/// Where a child's stdin comes from.
pub(crate) enum Input<'a> {
    Null,
    /// Shares the file object (and its position) with the caller's handle.
    File(&'a OwnedHandle),
    /// A pipe the caller writes to.
    Pipe,
}

/// Where a child's stdout goes (stderr is always a pipe).
pub(crate) enum Output<'a> {
    File(&'a OwnedHandle),
    /// A pipe the caller reads from.
    Pipe,
}

pub(crate) struct Process {
    pub pid: u32,
    pub handle: Arc<OwnedHandle>,
    pub stdin: Option<NamedPipeServer>,
    pub stdout: Option<NamedPipeServer>,
    pub stderr: NamedPipeServer,
}

fn pipe_security() -> io::Result<&'static UserOnlySecurity> {
    static SEC: OnceLock<Result<UserOnlySecurity, String>> = OnceLock::new();
    SEC.get_or_init(|| UserOnlySecurity::new(false).map_err(|e| e.to_string())).as_ref().map_err(|e| io::Error::other(e.clone()))
}

fn inheritable() -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: null_mut(), bInheritHandle: 1 }
}

/// An overlapped server end for us and an inheritable, synchronous client end for the child.
/// `child_writes`: the child writes (its stdout/stderr), otherwise it reads (its stdin).
///
/// The server end is handed to tokio only once the client end is connected: mio schedules the
/// first overlapped read at registration, and a pipe registered while still listening would
/// otherwise need a `connect()` that fails (`ERROR_NO_DATA`) if a short-lived child already exited.
fn pipe_pair(child_writes: bool) -> io::Result<(NamedPipeServer, OwnedHandle)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = wide(format!(r"\\.\pipe\lampo-stdio-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    let sec = pipe_security()?;
    let direction = if child_writes { PIPE_ACCESS_INBOUND } else { PIPE_ACCESS_OUTBOUND };
    // SAFETY: valid name and SECURITY_ATTRIBUTES that outlive the call.
    let server = owned(unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            direction | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64 * 1024,
            64 * 1024,
            0,
            sec.as_ptr(),
        )
    })?;
    let sa = inheritable();
    let access = if child_writes { GENERIC_WRITE } else { GENERIC_READ };
    // SAFETY: valid name and attributes; synchronous handle as console programs expect.
    let client = owned(unsafe { CreateFileW(name.as_ptr(), access, 0, &sa, OPEN_EXISTING, 0, null_mut()) })?;
    // SAFETY: an overlapped named-pipe server handle we own; ownership moves to tokio.
    let server = unsafe { NamedPipeServer::from_raw_handle(server.into_raw_handle())? };
    Ok((server, client))
}

fn null_device(write: bool) -> io::Result<OwnedHandle> {
    let sa = inheritable();
    let name = wide("NUL");
    let access = if write { GENERIC_WRITE } else { GENERIC_READ };
    // SAFETY: valid arguments.
    owned(unsafe { CreateFileW(name.as_ptr(), access, FILE_SHARE_READ | FILE_SHARE_WRITE, &sa, OPEN_EXISTING, 0, null_mut()) })
}

/// Inheritable duplicate sharing the same file object (and file position).
fn inheritable_dup(h: &OwnedHandle) -> io::Result<OwnedHandle> {
    let mut dup: HANDLE = null_mut();
    // SAFETY: valid source handle in our own process.
    let ok = unsafe { DuplicateHandle(GetCurrentProcess(), raw(h), GetCurrentProcess(), &mut dup, 0, 1, DUPLICATE_SAME_ACCESS) };
    if ok == 0 { Err(io::Error::last_os_error()) } else { owned(dup) }
}

/// Appends `arg` quoted per the MSVC CRT rules (`CommandLineToArgvW`).
pub(crate) fn push_arg(cmd: &mut Vec<u16>, arg: &OsStr, force_quotes: bool) {
    if !cmd.is_empty() {
        cmd.push(b' ' as u16);
    }
    let units: Vec<u16> = arg.encode_wide().collect();
    let quote = force_quotes || units.is_empty() || units.iter().any(|&c| c == b' ' as u16 || c == b'\t' as u16 || c == b'"' as u16);
    if !quote {
        cmd.extend_from_slice(&units);
        return;
    }
    cmd.push(b'"' as u16);
    let mut backslashes = 0usize;
    for &c in &units {
        if c == b'\\' as u16 {
            backslashes += 1;
        } else {
            if c == b'"' as u16 {
                cmd.extend(std::iter::repeat_n(b'\\' as u16, backslashes + 1));
            }
            backslashes = 0;
        }
        cmd.push(c);
    }
    cmd.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
    cmd.push(b'"' as u16);
}

pub(crate) fn command_line(spec: &CommandSpec) -> Vec<u16> {
    let mut cmd = Vec::new();
    push_arg(&mut cmd, spec.program.as_os_str(), true);
    for a in &spec.args {
        push_arg(&mut cmd, OsStr::new(a), false);
    }
    cmd.push(0);
    cmd
}

/// Inherited environment plus `extra` (case-insensitive override) and `path_prepend` in front of
/// the resulting `PATH`, sorted as Windows expects.
fn environment_block(extra: &[(String, String)], path_prepend: &[std::path::PathBuf]) -> Vec<u16> {
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    for (k, v) in extra {
        vars.retain(|(ek, _)| !ek.to_string_lossy().eq_ignore_ascii_case(k));
        vars.push((k.into(), v.into()));
    }
    if !path_prepend.is_empty() {
        let is_path = |k: &OsString| k.to_string_lossy().eq_ignore_ascii_case("PATH");
        let current = vars.iter().find(|(k, _)| is_path(k)).map(|(_, v)| v.clone()).unwrap_or_default();
        let mut path = OsString::new();
        for dir in path_prepend {
            path.push(dir.as_os_str());
            path.push(";");
        }
        path.push(&current);
        vars.retain(|(k, _)| !is_path(k));
        vars.push(("Path".into(), path));
    }
    vars.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (k, v) in vars {
        block.extend(k.encode_wide());
        block.push(b'=' as u16);
        block.extend(v.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

struct AttributeList(Vec<u8>);

impl AttributeList {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0usize;
        // SAFETY: size query with a null list.
        unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size) };
        let mut buf = vec![0u8; size];
        // SAFETY: buffer of the requested size.
        if unsafe { InitializeProcThreadAttributeList(buf.as_mut_ptr().cast(), count, 0, &mut size) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(buf))
    }

    /// `value` must stay alive and unmoved until the process is created.
    fn set<T>(&mut self, attribute: u32, value: &[T]) -> io::Result<()> {
        // SAFETY: initialised list; value pointer/size describe a live slice.
        let ok = unsafe { UpdateProcThreadAttribute(self.0.as_mut_ptr().cast(), 0, attribute as usize, value.as_ptr().cast(), size_of_val(value), null_mut(), null()) };
        if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: initialised in `new`.
        unsafe { DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast()) };
    }
}

/// Creates the process inside every job of `jobs` (in order: a later job is nested in the
/// earlier ones). Must run inside a tokio runtime (pipe registration).
pub(crate) fn spawn(spec: &CommandSpec, jobs: &[&OwnedHandle], stdin: Input<'_>, stdout: Output<'_>) -> io::Result<Process> {
    let (stdin_server, stdin_child) = match stdin {
        Input::Null => (None, null_device(false)?),
        Input::File(h) => (None, inheritable_dup(h)?),
        Input::Pipe => {
            let (s, c) = pipe_pair(false)?;
            (Some(s), c)
        }
    };
    let (stdout_server, stdout_child) = match stdout {
        Output::File(h) => (None, inheritable_dup(h)?),
        Output::Pipe => {
            let (s, c) = pipe_pair(true)?;
            (Some(s), c)
        }
    };
    let (stderr_server, stderr_child) = pipe_pair(true)?;

    let handles: [HANDLE; 3] = [raw(&stdin_child), raw(&stdout_child), raw(&stderr_child)];
    let jobs: Vec<HANDLE> = jobs.iter().map(|j| raw(j)).collect();
    let mut attributes = AttributeList::new(2)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &handles)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;

    // SAFETY: plain C struct, zero is a valid initial state.
    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    si.StartupInfo.hStdInput = handles[0];
    si.StartupInfo.hStdOutput = handles[1];
    si.StartupInfo.hStdError = handles[2];
    si.lpAttributeList = attributes.0.as_mut_ptr().cast();

    let mut cmd = command_line(spec);
    // An application name disables the PATH search: only pass it for explicit paths.
    let has_dir = spec.program.parent().is_some_and(|p| !p.as_os_str().is_empty());
    let app = has_dir.then(|| wide(&spec.program));
    let env = (!spec.env.is_empty() || !spec.path_prepend.is_empty()).then(|| environment_block(&spec.env, &spec.path_prepend));
    let cwd = spec.working_dir.as_ref().map(wide);
    // SAFETY: plain C struct.
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every pointer is valid for the duration of the call.
    let ok = unsafe {
        CreateProcessW(
            app.as_ref().map_or(null(), |a| a.as_ptr()),
            cmd.as_mut_ptr(),
            null(),
            null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            env.as_ref().map_or(null(), |e| e.as_ptr().cast()),
            cwd.as_ref().map_or(null(), |c| c.as_ptr()),
            &si.StartupInfo,
            &mut pi,
        )
    };
    if ok == 0 {
        let err = io::Error::last_os_error();
        return Err(io::Error::new(err.kind(), format!("cannot start `{}`: {err}", spec.program.display())));
    }
    win::close(pi.hThread);
    let handle = Arc::new(owned(pi.hProcess)?);
    // The child's ends are closed here (dropped), so EOF arrives when the child exits.
    drop((stdin_child, stdout_child, stderr_child));
    Ok(Process { pid: pi.dwProcessId, handle, stdin: stdin_server, stdout: stdout_server, stderr: stderr_server })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(args: &[&str]) -> String {
        let spec = CommandSpec { program: "C:\\bin\\p.exe".into(), args: args.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let mut v = command_line(&spec);
        v.pop();
        String::from_utf16(&v).unwrap()
    }

    #[test]
    fn quotes_like_the_crt() {
        assert_eq!(line(&["a", "b c", ""]), r#""C:\bin\p.exe" a "b c" """#);
        assert_eq!(line(&[r#"say "hi""#]), r#""C:\bin\p.exe" "say \"hi\"""#);
        assert_eq!(line(&[r"C:\dir with space\"]), r#""C:\bin\p.exe" "C:\dir with space\\""#);
        assert_eq!(line(&[r"--defaults-file=C:\x\my.ini"]), r#""C:\bin\p.exe" --defaults-file=C:\x\my.ini"#);
    }

    #[test]
    fn environment_overrides_case_insensitively() {
        let block = environment_block(&[("path".into(), "X".into())], &[]);
        let text = String::from_utf16_lossy(&block);
        let entries: Vec<&str> = text.split('\0').filter(|s| !s.is_empty()).collect();
        assert_eq!(entries.iter().filter(|e| e.to_ascii_lowercase().starts_with("path=")).count(), 1);
        assert!(entries.contains(&"path=X"));
    }

    #[test]
    fn path_prepend_goes_in_front_of_the_resulting_path() {
        let block = environment_block(&[("PATH".into(), "C:\\base".into())], &[std::path::PathBuf::from("C:\\first"), std::path::PathBuf::from("C:\\second")]);
        let text = String::from_utf16_lossy(&block);
        let paths: Vec<&str> = text.split('\0').filter(|e| e.to_ascii_lowercase().starts_with("path=")).collect();
        assert_eq!(paths, ["Path=C:\\first;C:\\second;C:\\base"]);
    }
}
