//! Changing Defender's exclusions from the elevated helper, through the Defender cmdlets of
//! Windows PowerShell (`Add-MpPreference` / `Remove-MpPreference`, `-ExclusionPath` for the
//! folder and `-ExclusionProcess` for the executables).
//!
//! No text is ever built from input: PowerShell is `<GetSystemDirectoryW()>\WindowsPowerShell\
//! v1.0\powershell.exe` (never found through PATH), the script is a constant, and the targets
//! reach it as environment variables (`$env:LAMPO_DEFENDER_FOLDER`, and
//! `$env:LAMPO_DEFENDER_PROCESSES` with one executable per line), which PowerShell reads as plain
//! strings. The child gets a minimal environment: the modules come only from System32, so nothing
//! the user can write (their PSModulePath, profile, temp folder) is loaded elevated.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lampo_domain::scan_exclusion::{ScanExclusionChange, ScanExclusionTargets};

use crate::system_dir::system_directory;

/// What Defender did with a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefenderResult {
    Applied,
    /// The cmdlets are missing, or Defender is not the active antivirus.
    Unavailable,
    /// The cmdlet failed (Tamper Protection, a policy).
    Refused,
    /// The cmdlet succeeded but the exclusion list does not show the change.
    NotConfirmed,
    /// PowerShell could not be run, did not finish in time or failed unexpectedly.
    Failed,
}

/// Applies a change to Defender's exclusions; `targets` are already validated.
pub trait DefenderSettings {
    fn apply(&self, change: ScanExclusionChange, targets: &ScanExclusionTargets) -> DefenderResult;
}

/// The variable carrying the folder to the scripts (absent: no folder).
const FOLDER_VAR: &str = "LAMPO_DEFENDER_FOLDER";
/// The variable carrying the executables, one per line (absent: none). Validated paths never
/// hold a control character, so the separator cannot appear inside one.
const PROCESSES_VAR: &str = "LAMPO_DEFENDER_PROCESSES";
const PROCESS_SEPARATOR: &str = "\n";

/// Exit codes of the scripts.
const SCRIPT_OK: i32 = 0;
const SCRIPT_UNAVAILABLE: i32 = 20;
const SCRIPT_REFUSED: i32 = 21;
const SCRIPT_NOT_CONFIRMED: i32 = 22;

/// Adds the exclusions only when Defender is the running antivirus (`AMRunningMode` is `Normal`,
/// not `Passive Mode` or `EDR Block Mode` next to another product), then reads them back.
const ADD_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
$folder = $env:LAMPO_DEFENDER_FOLDER
$processes = @(([string]$env:LAMPO_DEFENDER_PROCESSES).Split([char]10) | Where-Object { $_ })
if (-not (Get-Command Add-MpPreference -ErrorAction SilentlyContinue)) { exit 20 }
try { $status = Get-MpComputerStatus } catch { exit 20 }
if (-not $status.AntivirusEnabled -or $status.AMRunningMode -ne 'Normal') { exit 20 }
try {
  if ($folder) { Add-MpPreference -ExclusionPath $folder }
  if ($processes) { Add-MpPreference -ExclusionProcess $processes }
} catch { exit 21 }
try { $now = Get-MpPreference } catch { exit 22 }
if ($folder -and @($now.ExclusionPath) -notcontains $folder) { exit 22 }
foreach ($process in $processes) { if (@($now.ExclusionProcess) -notcontains $process) { exit 22 } }
exit 0"#;

/// Removes the exclusions whatever Defender's state (a passive Defender keeps its settings).
const REMOVE_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
$folder = $env:LAMPO_DEFENDER_FOLDER
$processes = @(([string]$env:LAMPO_DEFENDER_PROCESSES).Split([char]10) | Where-Object { $_ })
if (-not (Get-Command Remove-MpPreference -ErrorAction SilentlyContinue)) { exit 20 }
try {
  if ($folder) { Remove-MpPreference -ExclusionPath $folder }
  if ($processes) { Remove-MpPreference -ExclusionProcess $processes }
} catch { exit 21 }
try { $now = Get-MpPreference } catch { exit 22 }
if ($folder -and @($now.ExclusionPath) -contains $folder) { exit 22 }
foreach ($process in $processes) { if (@($now.ExclusionProcess) -contains $process) { exit 22 } }
exit 0"#;
/// The Defender cmdlets through Windows PowerShell.
pub struct PowerShellDefender {
    timeout: Duration,
}

impl PowerShellDefender {
    /// The cmdlets answer in seconds; a stuck Defender service must not hold the helper forever.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
    const POLL_INTERVAL: Duration = Duration::from_millis(100);
}

impl Default for PowerShellDefender {
    fn default() -> Self {
        Self { timeout: Self::DEFAULT_TIMEOUT }
    }
}

impl DefenderSettings for PowerShellDefender {
    fn apply(&self, change: ScanExclusionChange, targets: &ScanExclusionTargets) -> DefenderResult {
        let Some(mut command) = powershell_command(change, targets) else { return DefenderResult::Failed };
        match run_bounded(&mut command, self.timeout) {
            Some(code) => result_of(code),
            None => DefenderResult::Failed,
        }
    }
}

fn script(change: ScanExclusionChange) -> &'static str {
    match change {
        ScanExclusionChange::Add => ADD_SCRIPT,
        ScanExclusionChange::Remove => REMOVE_SCRIPT,
    }
}

fn result_of(code: i32) -> DefenderResult {
    match code {
        SCRIPT_OK => DefenderResult::Applied,
        SCRIPT_UNAVAILABLE => DefenderResult::Unavailable,
        SCRIPT_REFUSED => DefenderResult::Refused,
        SCRIPT_NOT_CONFIRMED => DefenderResult::NotConfirmed,
        _ => DefenderResult::Failed,
    }
}

/// `powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command <script>`
/// with a minimal environment and the targets. `None` when the system folder is unknown.
pub(crate) fn powershell_command(change: ScanExclusionChange, targets: &ScanExclusionTargets) -> Option<Command> {
    let system = system_directory()?;
    let windows = system.parent()?.to_path_buf();
    let powershell_dir = system.join("WindowsPowerShell").join("v1.0");
    let mut command = Command::new(powershell_dir.join("powershell.exe"));
    // `Bypass` concerns this process only and lets the signed cmdlet modules load whatever the
    // machine's default policy; the command itself is a constant, not a script file.
    command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script(change)]);
    command.env_clear().envs(minimal_environment(&windows, &powershell_dir));
    if let Some(folder) = &targets.folder {
        command.env(FOLDER_VAR, folder);
    }
    if !targets.processes.is_empty() {
        command.env(PROCESSES_VAR, process_list(&targets.processes));
    }
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    hide_console(&mut command);
    Some(command)
}

/// The executables, one per line.
fn process_list(processes: &[PathBuf]) -> std::ffi::OsString {
    let mut list = std::ffi::OsString::new();
    for (i, process) in processes.iter().enumerate() {
        if i > 0 {
            list.push(PROCESS_SEPARATOR);
        }
        list.push(process);
    }
    list
}

/// Just what PowerShell needs, from the system folders.
fn minimal_environment(windows: &Path, powershell_dir: &Path) -> Vec<(&'static str, PathBuf)> {
    let temp = windows.join("Temp");
    vec![("SystemRoot", windows.to_path_buf()), ("windir", windows.to_path_buf()), ("PSModulePath", powershell_dir.join("Modules")), ("TEMP", temp.clone()), ("TMP", temp)]
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

/// Runs `command` and returns its exit code; `None` when it cannot start, ends without a code, or
/// is still running after `timeout` (it is then killed).
fn run_bounded(command: &mut Command, timeout: Duration) -> Option<i32> {
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.code(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(PowerShellDefender::POLL_INTERVAL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_exit_codes_map_to_results() {
        assert_eq!(result_of(SCRIPT_OK), DefenderResult::Applied);
        assert_eq!(result_of(SCRIPT_UNAVAILABLE), DefenderResult::Unavailable);
        assert_eq!(result_of(SCRIPT_REFUSED), DefenderResult::Refused);
        assert_eq!(result_of(SCRIPT_NOT_CONFIRMED), DefenderResult::NotConfirmed);
        assert_eq!(result_of(1), DefenderResult::Failed, "a PowerShell error outside the script's own checks");
    }

    #[test]
    fn the_scripts_read_the_targets_only_from_the_environment() {
        for change in [ScanExclusionChange::Add, ScanExclusionChange::Remove] {
            let text = script(change);
            assert!(text.contains(&format!("$env:{FOLDER_VAR}")) && text.contains(&format!("$env:{PROCESSES_VAR}")));
            assert!(!text.contains('"'), "no interpolating strings");
            assert!(!text.contains('*'), "no wildcard");
            for code in [SCRIPT_OK, SCRIPT_UNAVAILABLE, SCRIPT_REFUSED, SCRIPT_NOT_CONFIRMED] {
                assert!(text.contains(&format!("exit {code}")), "{change:?} uses exit {code}");
            }
            assert!(text.contains("$now = Get-MpPreference"), "{change:?} reads the change back");
        }
        assert_eq!(PROCESS_SEPARATOR, "\n", "the scripts split on [char]10");
        for (script, cmdlet, other) in [(ADD_SCRIPT, "Add-MpPreference", "Remove-MpPreference"), (REMOVE_SCRIPT, "Remove-MpPreference", "Add-MpPreference")] {
            assert!(script.contains(&format!("{cmdlet} -ExclusionPath $folder")) && script.contains(&format!("{cmdlet} -ExclusionProcess $processes")));
            assert!(script.contains("@($now.ExclusionProcess)") && !script.contains(other));
        }
    }

    #[test]
    fn executables_are_one_per_line() {
        assert_eq!(process_list(&[PathBuf::from(r"C:\a\php-cgi.exe")]), r"C:\a\php-cgi.exe");
        assert_eq!(process_list(&[PathBuf::from(r"C:\a\php-cgi.exe"), PathBuf::from(r"C:\b\bin\httpd.exe")]), "C:\\a\\php-cgi.exe\nC:\\b\\bin\\httpd.exe");
    }

    #[cfg(windows)]
    fn environment(command: &Command) -> Vec<(String, Option<String>)> {
        command.get_envs().map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()))).collect()
    }

    #[cfg(windows)]
    #[test]
    fn powershell_comes_from_system32_with_a_minimal_environment() {
        let folder = PathBuf::from(r"C:\Users\me\AppData\Local\Lampo\mysql");
        let php = PathBuf::from(r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe");
        let targets = ScanExclusionTargets { folder: Some(folder.clone()), processes: vec![php.clone()] };
        let command = powershell_command(ScanExclusionChange::Add, &targets).unwrap();
        let system = system_directory().unwrap();
        assert_eq!(Path::new(command.get_program()), system.join(r"WindowsPowerShell\v1.0\powershell.exe"));
        let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args.last().map(String::as_str), Some(ADD_SCRIPT));
        assert!(!args.iter().any(|a| a.contains(r"Lampo\")), "no target is ever on the command line");
        let env = environment(&command);
        assert!(env.contains(&(FOLDER_VAR.to_string(), Some(folder.display().to_string()))));
        assert!(env.contains(&(PROCESSES_VAR.to_string(), Some(php.display().to_string()))));
        assert!(env.contains(&("PSModulePath".to_string(), Some(system.join(r"WindowsPowerShell\v1.0\Modules").display().to_string()))));
    }

    #[cfg(windows)]
    #[test]
    fn absent_targets_set_no_variable() {
        let processes_only = ScanExclusionTargets { folder: None, processes: vec![PathBuf::from(r"C:\a\php-cgi.exe")] };
        let names: Vec<String> = environment(&powershell_command(ScanExclusionChange::Remove, &processes_only).unwrap()).into_iter().map(|(k, _)| k).collect();
        assert!(!names.contains(&FOLDER_VAR.to_string()) && names.contains(&PROCESSES_VAR.to_string()), "{names:?}");
        let folder_only = ScanExclusionTargets { folder: Some(PathBuf::from(r"C:\a")), processes: Vec::new() };
        let names: Vec<String> = environment(&powershell_command(ScanExclusionChange::Remove, &folder_only).unwrap()).into_iter().map(|(k, _)| k).collect();
        assert!(names.contains(&FOLDER_VAR.to_string()) && !names.contains(&PROCESSES_VAR.to_string()), "{names:?}");
    }

    #[cfg(windows)]
    #[test]
    fn a_command_that_outlives_the_timeout_is_stopped() {
        let mut command = Command::new(system_directory().unwrap().join("ping.exe"));
        command.args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null());
        assert_eq!(run_bounded(&mut command, Duration::from_millis(200)), None);
    }
}
