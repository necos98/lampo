//! Thin, safe wrappers over the Win32 calls the adapters share: UTF-16 strings, the current
//! user's SID, a "current user only" security descriptor, process-exit waits on the system
//! thread pool, process names and the TCP listener table. No adapter logic lives here.

use std::ffi::{OsStr, c_void};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};

use tokio::sync::oneshot;
use windows_sys::Win32::Foundation::{BOOLEAN, CloseHandle, ERROR_INSUFFICIENT_BUFFER, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree, NO_ERROR, SYSTEMTIME, WAIT_OBJECT_0};
use windows_sys::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, MIB_TCP6TABLE_OWNER_PID, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, RegisterWaitForSingleObject, UnregisterWaitEx, WT_EXECUTEONLYONCE, WaitForSingleObject,
};
use windows_sys::Win32::System::Time::TzSpecificLocalTimeToSystemTime;

/// NUL-terminated UTF-16.
pub(crate) fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

pub(crate) fn raw(h: &OwnedHandle) -> HANDLE {
    h.as_raw_handle() as HANDLE
}

/// Takes ownership of a handle returned by a Win32 call (NULL / INVALID_HANDLE_VALUE â†’ error).
pub(crate) fn owned(h: HANDLE) -> io::Result<OwnedHandle> {
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: a valid handle freshly returned to us; ownership is transferred.
        Ok(unsafe { OwnedHandle::from_raw_handle(h as RawHandle) })
    }
}

struct LocalMem(*mut c_void);

impl Drop for LocalMem {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: memory allocated by the system with LocalAlloc.
            unsafe { LocalFree(self.0 as HLOCAL) };
        }
    }
}

/// The current process user's SID (owned bytes, valid `PSID` via `as_ptr`).
struct UserSid {
    buffer: Vec<u64>,
}

impl UserSid {
    fn current() -> io::Result<Self> {
        unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let token = owned(token)?;
            let mut len = 0u32;
            GetTokenInformation(raw(&token), TokenUser, null_mut(), 0, &mut len);
            if len == 0 {
                return Err(io::Error::last_os_error());
            }
            // u64 storage keeps the TOKEN_USER properly aligned.
            let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
            if GetTokenInformation(raw(&token), TokenUser, buffer.as_mut_ptr().cast(), len, &mut len) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { buffer })
        }
    }

    fn psid(&self) -> PSID {
        // SAFETY: the buffer holds a TOKEN_USER whose SID points inside the same buffer.
        unsafe { (*(self.buffer.as_ptr() as *const TOKEN_USER)).User.Sid }
    }

    fn to_string_sid(&self) -> io::Result<String> {
        unsafe {
            let mut s: *mut u16 = null_mut();
            if ConvertSidToStringSidW(self.psid(), &mut s) == 0 {
                return Err(io::Error::last_os_error());
            }
            let _free = LocalMem(s.cast());
            let len = (0..).take_while(|&i| *s.add(i) != 0).count();
            Ok(String::from_utf16_lossy(std::slice::from_raw_parts(s, len)))
        }
    }
}

/// String form (`S-1-5-21-â€¦`) of the current user's SID, computed once.
pub(crate) fn current_user_sid() -> io::Result<&'static str> {
    static SID: OnceLock<Result<String, String>> = OnceLock::new();
    SID.get_or_init(|| UserSid::current().and_then(|s| s.to_string_sid()).map_err(|e| e.to_string())).as_deref().map_err(|e| io::Error::other(e.clone()))
}

/// `SECURITY_ATTRIBUTES` whose descriptor is owned by, and grants access only to, the current
/// user (protected DACL: nothing inherited, no Everyone/Administrators/SYSTEM entries).
pub(crate) struct UserOnlySecurity {
    sd: LocalMem,
    attributes: Box<SECURITY_ATTRIBUTES>,
}

// SAFETY: the descriptor is immutable after construction and only read by the kernel.
unsafe impl Send for UserOnlySecurity {}
unsafe impl Sync for UserOnlySecurity {}

impl UserOnlySecurity {
    pub(crate) fn new(inheritable: bool) -> io::Result<Self> {
        let sid = current_user_sid()?;
        Self::from_sddl(&format!("O:{sid}D:P(A;;GA;;;{sid})"), inheritable)
    }

    /// Attributes for an arbitrary SDDL descriptor. Production code uses [`Self::new`]; tests use
    /// this to build objects Lampo itself never creates (e.g. owned by someone else).
    pub(crate) fn from_sddl(sddl: &str, inheritable: bool) -> io::Result<Self> {
        let sddl = wide(sddl);
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: valid NUL-terminated SDDL; the result is freed with LocalFree.
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let attributes = Box::new(SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: i32::from(inheritable) });
        Ok(Self { sd: LocalMem(sd), attributes })
    }

    pub(crate) fn as_ptr(&self) -> *const SECURITY_ATTRIBUTES {
        debug_assert!(!self.sd.0.is_null());
        &*self.attributes
    }
}

/// Whether the kernel object behind `handle` is owned by the current user.
///
/// "Current user" is the token's `TokenUser` SID, the same SID [`UserOnlySecurity`] writes as the
/// explicit owner, not `TokenOwner` (the default owner, BUILTIN\Administrators in an elevated
/// token). An elevated and a non-elevated process of the same user therefore agree.
pub(crate) fn is_owned_by_current_user(handle: HANDLE) -> io::Result<bool> {
    let me = UserSid::current()?;
    unsafe {
        let mut owner: PSID = null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        let rc = GetSecurityInfo(handle, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, &mut owner, null_mut(), null_mut(), null_mut(), &mut sd);
        if rc != NO_ERROR {
            return Err(io::Error::from_raw_os_error(rc as i32));
        }
        let _free = LocalMem(sd);
        Ok(!owner.is_null() && EqualSid(owner, me.psid()) != 0)
    }
}

/// Resolves once `process` has exited. Uses a system thread-pool wait
/// (`RegisterWaitForSingleObject`): no thread is parked per child, nothing polls.
pub(crate) async fn wait_for_exit(process: &OwnedHandle) -> io::Result<()> {
    type Slot = Mutex<Option<oneshot::Sender<()>>>;

    unsafe extern "system" fn on_signal(context: *mut c_void, _timed_out: BOOLEAN) {
        // SAFETY: `context` is the boxed slot, alive until the wait is unregistered (Guard).
        let slot = unsafe { &*(context as *const Slot) };
        if let Some(tx) = slot.lock().ok().and_then(|mut s| s.take()) {
            let _ = tx.send(());
        }
    }

    struct Guard {
        wait: usize,
        slot: *mut Slot,
    }
    // SAFETY: the wait handle and slot pointer are only used in Drop.
    unsafe impl Send for Guard {}
    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: INVALID_HANDLE_VALUE makes UnregisterWaitEx wait for a running callback,
            // so the slot can be freed afterwards.
            unsafe {
                UnregisterWaitEx(self.wait as HANDLE, INVALID_HANDLE_VALUE);
                drop(Box::from_raw(self.slot));
            }
        }
    }

    let (tx, rx) = oneshot::channel();
    // Raw pointers stay inside this block so the future remains `Send`.
    let guard = {
        let slot: *mut Slot = Box::into_raw(Box::new(Mutex::new(Some(tx))));
        let mut wait: HANDLE = null_mut();
        // SAFETY: valid process handle and context pointer (freed by Guard / below on failure).
        let ok = unsafe { RegisterWaitForSingleObject(&mut wait, raw(process), Some(on_signal), slot as *const c_void, INFINITE, WT_EXECUTEONLYONCE) };
        if ok == 0 {
            let err = io::Error::last_os_error();
            // SAFETY: registration failed, nobody else references the slot.
            drop(unsafe { Box::from_raw(slot) });
            return Err(err);
        }
        Guard { wait: wait as usize, slot }
    };
    let result = rx.await.map_err(|_| io::Error::other("process wait dropped"));
    drop(guard);
    result
}

pub(crate) fn has_exited(process: &OwnedHandle) -> bool {
    // SAFETY: valid handle; zero timeout.
    unsafe { WaitForSingleObject(raw(process), 0) == WAIT_OBJECT_0 }
}

pub(crate) fn exit_code(process: &OwnedHandle) -> Option<i32> {
    let mut code = 0u32;
    // SAFETY: valid handle and out pointer.
    (unsafe { GetExitCodeProcess(raw(process), &mut code) } != 0).then_some(code as i32)
}

/// Executable names of running processes by pid (one Toolhelp snapshot; works for elevated
/// and service processes too, unlike `OpenProcess`).
pub(crate) fn process_name(pid: u32) -> Option<String> {
    match pid {
        0 => return Some("System Idle Process".into()),
        4 => return Some("System".into()),
        _ => {}
    }
    unsafe {
        let snap = owned(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)).ok()?;
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(raw(&snap), &mut entry) != 0;
        while ok {
            if entry.th32ProcessID == pid {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                return Some(String::from_utf16_lossy(&entry.szExeFile[..len]));
            }
            ok = Process32NextW(raw(&snap), &mut entry) != 0;
        }
    }
    None
}

/// Whether `pid` is `ancestor` or one of its descendants (parent chain from one snapshot).
pub(crate) fn is_same_or_descendant(pid: u32, ancestor: u32) -> bool {
    if pid == ancestor {
        return true;
    }
    let mut parents = std::collections::HashMap::new();
    unsafe {
        let Ok(snap) = owned(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)) else { return false };
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(raw(&snap), &mut entry) != 0;
        while ok {
            parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
            ok = Process32NextW(raw(&snap), &mut entry) != 0;
        }
    }
    let mut current = pid;
    for _ in 0..16 {
        match parents.get(&current) {
            Some(&parent) if parent == ancestor => return true,
            Some(&parent) if parent != 0 && parent != current => current = parent,
            _ => return false,
        }
    }
    false
}

/// `name.exe (pid 42)`, or `pid 42` when the process is gone.
pub(crate) fn describe_process(pid: u32) -> String {
    match process_name(pid) {
        Some(name) => format!("{name} (pid {pid})"),
        None => format!("pid {pid}"),
    }
}

fn tcp_table(family: u32) -> io::Result<Vec<u64>> {
    let mut size = 0u32;
    let mut buffer: Vec<u64> = Vec::new();
    for _ in 0..8 {
        // SAFETY: buffer of `size` bytes (or null with size 0 to query the size).
        let rc = unsafe { GetExtendedTcpTable(buffer.as_mut_ptr().cast(), &mut size, 0, family, TCP_TABLE_OWNER_PID_LISTENER, 0) };
        match rc {
            NO_ERROR => return Ok(buffer),
            ERROR_INSUFFICIENT_BUFFER => buffer = vec![0u64; (size as usize).div_ceil(8) + 8],
            other => return Err(io::Error::from_raw_os_error(other as i32)),
        }
        size = (buffer.len() * 8) as u32;
    }
    Err(io::Error::other("TCP table kept growing"))
}

/// Pid of a process listening on TCP `port` (IPv4 or IPv6, any local address).
pub(crate) fn tcp_listener_pid(port: u16) -> io::Result<Option<u32>> {
    let v4 = tcp_table(AF_INET as u32)?;
    // SAFETY: the buffer holds a MIB_TCPTABLE_OWNER_PID with dwNumEntries rows.
    unsafe {
        let table = v4.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
        let rows = std::slice::from_raw_parts((*table).table.as_ptr(), (*table).dwNumEntries as usize);
        if let Some(row) = rows.iter().find(|r| u16::from_be(r.dwLocalPort as u16) == port) {
            return Ok(Some(row.dwOwningPid));
        }
    }
    let v6 = tcp_table(AF_INET6 as u32)?;
    // SAFETY: as above, IPv6 layout.
    unsafe {
        let table = v6.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID;
        let rows = std::slice::from_raw_parts((*table).table.as_ptr(), (*table).dwNumEntries as usize);
        Ok(rows.iter().find(|r| u16::from_be(r.dwLocalPort as u16) == port).map(|r| r.dwOwningPid))
    }
}

/// Converts a local wall-clock time (current Windows time zone rules for that date, DST
/// included) to Unix milliseconds.
pub(crate) fn local_to_unix_ms(year: u16, month: u16, day: u16, hour: u16, minute: u16, second: u16, millis: u16) -> Option<u64> {
    let local = SYSTEMTIME { wYear: year, wMonth: month, wDayOfWeek: 0, wDay: day, wHour: hour, wMinute: minute, wSecond: second, wMilliseconds: millis };
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: valid in/out structs; NULL = current time zone.
    if unsafe { TzSpecificLocalTimeToSystemTime(null(), &local, &mut utc) } == 0 {
        return None;
    }
    crate::logs::civil_to_unix_ms(utc.wYear as i64, utc.wMonth as u32, utc.wDay as u32, utc.wHour as u32, utc.wMinute as u32, utc.wSecond as u32, utc.wMilliseconds as u32)
}

/// Closes a raw handle we own but have not wrapped.
pub(crate) fn close(h: HANDLE) {
    if !h.is_null() && h != INVALID_HANDLE_VALUE {
        // SAFETY: caller passes a handle it owns.
        unsafe { CloseHandle(h) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_object_created_with_user_only_security_is_owned_by_the_current_user() {
        let security = UserOnlySecurity::new(false).unwrap();
        let name = format!(r"\\.\pipe\lampo-win-test-{}", std::process::id());
        // SAFETY: valid SECURITY_ATTRIBUTES alive for the call.
        let pipe = unsafe { tokio::net::windows::named_pipe::ServerOptions::new().create_with_security_attributes_raw(&name, security.as_ptr() as *mut c_void) }.unwrap();
        assert!(is_owned_by_current_user(pipe.as_raw_handle() as HANDLE).unwrap());
    }

    #[test]
    fn an_object_owned_by_someone_else_is_not_owned_by_the_current_user() {
        // System files are owned by TrustedInstaller, whether or not this process is elevated.
        let system = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let file = std::fs::File::open(std::path::Path::new(&system).join(r"System32\kernel32.dll")).unwrap();
        assert!(!is_owned_by_current_user(file.as_raw_handle() as HANDLE).unwrap());
    }
}
