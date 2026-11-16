//! Launching external desktop apps (PowerShell, Explorer, VS Code) on a project folder.
//!
//! These are ordinary child processes of the caller: the desktop app process itself is not in
//! the services Job Object (services are created *inside* the job via
//! `PROC_THREAD_ATTRIBUTE_JOB_LIST`, see the `process` module), so nothing started here is killed
//! when Lampo quits.

use std::ffi::OsStr;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use lampo_domain::ports::ExternalAppLauncher;
use lampo_domain::project::ProjectId;
use lampo_domain::{PortError, PortResult};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
const EXPLORER_EXE: &str = "explorer.exe";
const CMD_EXE: &str = "cmd.exe";
const POWERSHELL_EXE: &str = "powershell.exe";
const LAMPO_EXE: &str = "lampo.exe";
const VSCODE_DIR: &str = "Microsoft VS Code";
const VSCODE_EXE: &str = "Code.exe";
const VSCODE_CMD: &str = "code.cmd";
const VSCODE_ENTITY: &str = "Visual Studio Code";
/// `cmd.exe` expands `%VAR%` even inside quotes, so such a folder cannot be passed through it safely.
const CMD_EXPANSION_CHAR: char = '%';

/// [`ExternalAppLauncher`] for Windows: Explorer, VS Code (`Code.exe`, or `code.cmd` from PATH)
/// and a PowerShell console through `lampo exec`.
pub struct WindowsExternalAppLauncher;

impl ExternalAppLauncher for WindowsExternalAppLauncher {
    /// Explorer exits with 1 even on success, so it is not waited on.
    fn open_folder(&self, folder: &Path) -> PortResult<()> {
        Command::new(EXPLORER_EXE).arg(folder).spawn().map_err(|e| PortError::process(EXPLORER_EXE, e))?;
        Ok(())
    }

    fn open_in_vscode(&self, folder: &Path) -> PortResult<()> {
        let local_app_data = std::env::var_os("LOCALAPPDATA");
        let program_files = std::env::var_os("ProgramFiles");
        let program_files_x86 = std::env::var_os("ProgramFiles(x86)");
        let path = std::env::var_os("PATH");
        let search =
            VsCodeSearch { local_app_data: local_app_data.as_deref(), program_files: program_files.as_deref(), program_files_x86: program_files_x86.as_deref(), path: path.as_deref() };
        let Some(vscode) = locate_vscode(search, Path::is_file) else {
            return Err(PortError::not_found(VSCODE_ENTITY, VSCODE_CMD));
        };
        let mut command = match &vscode {
            VsCode::Exe(exe) => {
                let mut c = Command::new(exe);
                c.arg(folder);
                c
            }
            VsCode::Cmd(script) => {
                ensure_cmd_safe(folder)?;
                // `cmd /s /c ""<script>" "<folder>""`: with /s cmd strips exactly the outer quote pair,
                // so both paths keep their own quotes (and may contain spaces). Windows paths cannot
                // contain `"`; `%` is refused above and `/v:off` keeps `!` literal.
                let mut c = Command::new(CMD_EXE);
                c.args(["/d", "/v:off", "/s", "/c"]).raw_arg(format!("\"\"{}\" \"{}\"\"", script.display(), folder.display()));
                c
            }
        };
        command.current_dir(folder).creation_flags(CREATE_NO_WINDOW).spawn().map_err(|e| PortError::process(VSCODE_ENTITY, e))?;
        Ok(())
    }

    /// `lampo.exe` sits next to the caller's executable (the desktop app or the CLI itself).
    fn open_terminal(&self, project: &ProjectId, folder: &Path) -> PortResult<()> {
        let lampo = std::env::current_exe().map_err(|e| PortError::io("locating the lampo CLI", e))?.with_file_name(LAMPO_EXE);
        Command::new(lampo)
            .args(["-p", project.as_str(), "exec", "--", POWERSHELL_EXE, "-NoLogo", "-NoExit"])
            .current_dir(folder)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map_err(|e| PortError::process(LAMPO_EXE, e))?;
        Ok(())
    }
}

/// How VS Code can be started.
#[derive(Debug, Clone, PartialEq, Eq)]
enum VsCode {
    /// `Code.exe` from a standard install location: started directly.
    Exe(PathBuf),
    /// `code.cmd` found on PATH with no `Code.exe` beside it: started through `cmd.exe`.
    Cmd(PathBuf),
}

/// Where to look for VS Code; the values of the corresponding environment variables.
#[derive(Debug, Default, Clone, Copy)]
struct VsCodeSearch<'a> {
    local_app_data: Option<&'a OsStr>,
    program_files: Option<&'a OsStr>,
    program_files_x86: Option<&'a OsStr>,
    path: Option<&'a OsStr>,
}

/// Finds VS Code: user install, machine install (64-bit, then 32-bit), then `code.cmd` on PATH,
/// preferring the `Code.exe` of that install (`<install>\bin\code.cmd` → `<install>\Code.exe`).
/// `is_file` is injected so the search order can be tested without touching the disk.
fn locate_vscode(search: VsCodeSearch<'_>, is_file: impl Fn(&Path) -> bool) -> Option<VsCode> {
    let installs = [
        search.local_app_data.map(|d| Path::new(d).join("Programs").join(VSCODE_DIR).join(VSCODE_EXE)),
        search.program_files.map(|d| Path::new(d).join(VSCODE_DIR).join(VSCODE_EXE)),
        search.program_files_x86.map(|d| Path::new(d).join(VSCODE_DIR).join(VSCODE_EXE)),
    ];
    if let Some(exe) = installs.into_iter().flatten().find(|p| is_file(p)) {
        return Some(VsCode::Exe(exe));
    }
    let path = search.path?;
    let script = std::env::split_paths(path).filter(|d| !d.as_os_str().is_empty()).map(|d| d.join(VSCODE_CMD)).find(|p| is_file(p))?;
    Some(match exe_beside_script(&script).filter(|exe| is_file(exe)) {
        Some(exe) => VsCode::Exe(exe),
        None => VsCode::Cmd(script),
    })
}

/// `<install>\Code.exe` for a `<install>\bin\code.cmd` launcher.
fn exe_beside_script(script: &Path) -> Option<PathBuf> {
    Some(script.parent()?.parent()?.join(VSCODE_EXE))
}

/// Refuses a folder that `cmd.exe` would rewrite: `%VAR%` expands even between quotes.
fn ensure_cmd_safe(folder: &Path) -> PortResult<()> {
    if folder.as_os_str().to_string_lossy().contains(CMD_EXPANSION_CHAR) {
        return Err(PortError::Unsupported(format!(
            "impossibile aprire in Visual Studio Code una cartella con `{CMD_EXPANSION_CHAR}` nel percorso tramite `code.cmd`: installa VS Code nel percorso standard o rinomina la cartella ({})",
            folder.display()
        )));
    }
    Ok(())
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
        let found = locate_vscode(s, |p| (p.starts_with(r"C:\Tools") || p.starts_with(r"C:\Other")) && p.ends_with(VSCODE_CMD));
        assert_eq!(found, Some(VsCode::Cmd(PathBuf::from(r"C:\Tools\VS Code\bin\code.cmd"))));
    }

    #[test]
    fn prefers_code_exe_beside_code_cmd_on_path() {
        let path = OsString::from(r"C:\Tools\VS Code\bin");
        let s = VsCodeSearch { path: Some(&path), ..Default::default() };
        let found = locate_vscode(s, |_| true);
        assert_eq!(found, Some(VsCode::Exe(PathBuf::from(r"C:\Tools\VS Code\Code.exe"))));
    }

    #[test]
    fn code_cmd_only_when_no_code_exe_beside_it() {
        let path = OsString::from(r"C:\Tools\VS Code\bin");
        let s = VsCodeSearch { path: Some(&path), ..Default::default() };
        let found = locate_vscode(s, |p| p.ends_with(VSCODE_CMD));
        assert_eq!(found, Some(VsCode::Cmd(PathBuf::from(r"C:\Tools\VS Code\bin\code.cmd"))));
    }

    #[test]
    fn cmd_launch_refuses_folders_with_percent() {
        let err = ensure_cmd_safe(Path::new(r"C:\www\%USERPROFILE%\app")).unwrap_err();
        assert!(matches!(&err, PortError::Unsupported(reason) if reason.ends_with(r"(C:\www\%USERPROFILE%\app)")), "{err:?}");
        assert!(ensure_cmd_safe(Path::new(r"C:\www\my app (1) & co")).is_ok());
    }

    #[test]
    fn none_when_not_installed() {
        let path = OsString::from(r"C:\Windows");
        let s = VsCodeSearch { path: Some(&path), ..Default::default() };
        assert_eq!(locate_vscode(s, |_| false), None);
        assert_eq!(locate_vscode(VsCodeSearch::default(), |_| true), None);
    }
}
