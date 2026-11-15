//! Process inspection helpers shared by the integration tests.
#![allow(dead_code)]

use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};

/// SYNCHRONIZE handle to a process (so pid reuse cannot fool the check).
pub struct ProcHandle(HANDLE);

unsafe impl Send for ProcHandle {}

impl ProcHandle {
    pub fn open(pid: u32) -> Self {
        let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!h.is_null(), "cannot open pid {pid}");
        Self(h)
    }

    pub fn exits_within(&self, d: Duration) -> bool {
        unsafe { WaitForSingleObject(self.0, d.as_millis() as u32) == WAIT_OBJECT_0 }
    }
}

impl Drop for ProcHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Pids of running processes whose parent is `parent` and whose image is `exe`.
pub fn children_named(parent: u32, exe: &str) -> Vec<u32> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
            if e.th32ParentProcessID == parent && String::from_utf16_lossy(&e.szExeFile[..len]).eq_ignore_ascii_case(exe) {
                out.push(e.th32ProcessID);
            }
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}
