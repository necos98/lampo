//! Launching external desktop apps (Explorer, VS Code) on a project folder.
//!
//! These are ordinary child processes of the app: the app process itself is not in the services
//! Job Object (services are created *inside* the job via `PROC_THREAD_ATTRIBUTE_JOB_LIST`, see
//! `lampo_infra_services::process`), so nothing started here is killed when Lampo quits.

use std::ffi::OsStr;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use lampo_application::{AppError, AppResult, ErrorCode};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const VSCODE_DIR: &str = "Microsoft VS Code";

/// How VS Code can be started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VsCode {
    /// `Code.exe` from a standard install location: started directly.
    Exe(PathBuf),
    /// `code.cmd` found on PATH: started through `cmd.exe`.
    Cmd(PathBuf),
}

/// Where to look for VS Code; the values of the corresponding environment variables.
#[derive(Debug, Default, Clone, Copy)]
pub struct VsCodeSearch<'a> {
    pub local_app_data: Option<&'a OsStr>,
    pub program_files: Option<&'a OsStr>,
    pub program_files_x86: Option<&'a OsStr>,
    pub path: Option<&'a OsStr>,
}

/// Finds VS Code: user install, machine install (64-bit, then 32-bit), then `code.cmd` on PATH.
/// `is_file` is injected so the search order can be tested without touching the disk.
pub fn locate_vscode(search: VsCodeSearch<'_>, is_file: impl Fn(&Path) -> bool) -> Option<VsCode> {
    let installs = [
        search.local_app_data.map(|d| Path::new(d).join("Programs").join(VSCODE_DIR).join("Code.exe")),
        search.program_files.map(|d| Path::new(d).join(VSCODE_DIR).join("Code.exe")),
        search.program_files_x86.map(|d| Path::new(d).join(VSCODE_DIR).join("Code.exe")),
    ];
    if let Some(exe) = installs.into_iter().flatten().find(|p| is_file(p)) {
        return Some(VsCode::Exe(exe));
    }
    let path = search.path?;
    std::env::split_paths(path).filter(|d| !d.as_os_str().is_empty()).map(|d| d.join("code.cmd")).find(|p| is_file(p)).map(VsCode::Cmd)
}

/// Opens `folder` in Windows Explorer. Explorer exits with 1 even on success, so it is not waited on.
pub fn open_folder(folder: &str) -> AppResult<()> {
    ensure_folder(folder)?;
    Command::new("explorer.exe").arg(folder).spawn().map_err(|e| AppError::internal(format!("impossibile aprire la cartella: {e}")))?;
    Ok(())
}

/// Opens `folder` in Visual Studio Code.
pub fn open_in_vscode(folder: &str) -> AppResult<()> {
    ensure_folder(folder)?;
    let local_app_data = std::env::var_os("LOCALAPPDATA");
    let program_files = std::env::var_os("ProgramFiles");
    let program_files_x86 = std::env::var_os("ProgramFiles(x86)");
    let path = std::env::var_os("PATH");
    let search = VsCodeSearch { local_app_data: local_app_data.as_deref(), program_files: program_files.as_deref(), program_files_x86: program_files_x86.as_deref(), path: path.as_deref() };
    let Some(vscode) = locate_vscode(search, Path::is_file) else {
        return Err(AppError::new(ErrorCode::PackageMissing, "Visual Studio Code non trovato: installalo o aggiungi `code` al PATH"));
    };
    let mut command = match &vscode {
        VsCode::Exe(exe) => {
            let mut c = Command::new(exe);
            c.arg(folder);
            c
        }
        VsCode::Cmd(script) => {
            // `cmd /s /c ""<script>" "<folder>""`: with /s cmd strips exactly the outer quote pair,
            // so both paths keep their own quotes (and may contain spaces). Windows paths cannot
            // contain `"`.
            let mut c = Command::new("cmd.exe");
            c.args(["/d", "/s", "/c"]).raw_arg(format!("\"\"{}\" \"{}\"\"", script.display(), folder));
            c
        }
    };
    command.current_dir(folder).creation_flags(CREATE_NO_WINDOW).spawn().map_err(|e| AppError::internal(format!("impossibile avviare Visual Studio Code: {e}")))?;
    Ok(())
}

fn ensure_folder(folder: &str) -> AppResult<()> {
    if Path::new(folder).is_dir() { Ok(()) } else { Err(AppError::new(ErrorCode::Validation, format!("la cartella del progetto non esiste: {folder}"))) }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    fn search<'a>(local: &'a OsStr, pf: &'a OsStr, pf86: &'a OsStr, path: &'a OsStr) -> VsCodeSearch<'a> {
        VsCodeSearch { local_app_data: Some(local), program_files: Some(pf), program_files_x86: Some(pf86), path: Some(path) }
    }

    #[test]
    fn prefers_user_install_then_machine_installs() {
        let (local, pf, pf86, path) =
            (OsString::from(r"C:\Users\u\AppData\Local"), OsString::from(r"C:\Program Files"), OsString::from(r"C:\Program Files (x86)"), OsString::from(r"C:\bin"));
        let s = search(&local, &pf, &pf86, &path);

        let all = locate_vscode(s, |_| true);
        assert_eq!(all, Some(VsCode::Exe(PathBuf::from(r"C:\Users\u\AppData\Local\Programs\Microsoft VS Code\Code.exe"))));

        let machine = locate_vscode(s, |p| p.starts_with(r"C:\Program Files"));
        assert_eq!(machine, Some(VsCode::Exe(PathBuf::from(r"C:\Program Files\Microsoft VS Code\Code.exe"))));

        let x86 = locate_vscode(s, |p| p.starts_with(r"C:\Program Files (x86)"));
        assert_eq!(x86, Some(VsCode::Exe(PathBuf::from(r"C:\Program Files (x86)\Microsoft VS Code\Code.exe"))));
    }

    #[test]
    fn falls_back_to_code_cmd_on_path() {
        let path = OsString::from(r"C:\Windows;;C:\Tools\VS Code\bin;C:\Other\bin");
        let s = VsCodeSearch { path: Some(&path), ..Default::default() };
        let found = locate_vscode(s, |p| p.starts_with(r"C:\Tools") || p.starts_with(r"C:\Other"));
        assert_eq!(found, Some(VsCode::Cmd(PathBuf::from(r"C:\Tools\VS Code\bin\code.cmd"))));
    }

    #[test]
    fn none_when_not_installed() {
        let path = OsString::from(r"C:\Windows");
        let s = VsCodeSearch { path: Some(&path), ..Default::default() };
        assert_eq!(locate_vscode(s, |_| false), None);
        assert_eq!(locate_vscode(VsCodeSearch::default(), |_| true), None);
    }
}
