//! Reading Microsoft Defender's state as the user, without elevation (`Get-MpComputerStatus`,
//! which a standard user may call, unlike the exclusion list of `Get-MpPreference`).
//!
//! The same safe launch as the helper's cmdlets ([`super::powershell::system_powershell`]):
//! PowerShell from System32, a constant script that reads no input, a minimal environment. The
//! answer is the exit code; a stuck Defender service costs at most [`PowerShellDefenderState::TIMEOUT`].

use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::ports::DefenderStateReader;
use lampo_domain::scan_exclusion::DefenderState;
use lampo_domain::{PortError, PortResult};

use super::powershell::{run_bounded, system_powershell};

const PROGRAM: &str = "powershell";

/// Exit codes of [`STATE_SCRIPT`].
const REAL_TIME_PROTECTION: i32 = 0;
const UNAVAILABLE: i32 = 20;
const PASSIVE: i32 = 21;
const REAL_TIME_PROTECTION_OFF: i32 = 22;

/// Unavailable without the cmdlets, when the status cannot be read (the Defender service is off
/// next to another antivirus) or the antivirus is disabled; passive in any running mode other
/// than `Normal` (`Passive Mode`, `SxS Passive Mode`, `EDR Block Mode`). Older Windows builds
/// have no `AMRunningMode`: Defender is then the active antivirus.
const STATE_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
if (-not (Get-Command Get-MpComputerStatus -ErrorAction SilentlyContinue)) { exit 20 }
try { $status = Get-MpComputerStatus } catch { exit 20 }
if (-not $status.AntivirusEnabled) { exit 20 }
$mode = [string]$status.AMRunningMode
if ($mode -eq 'Not running') { exit 20 }
if ($mode -and $mode -ne 'Normal') { exit 21 }
if (-not $status.RealTimeProtectionEnabled) { exit 22 }
exit 0"#;

/// The `DefenderStateReader` adapter: [`STATE_SCRIPT`] through Windows PowerShell.
pub struct PowerShellDefenderState {
    timeout: Duration,
}

impl PowerShellDefenderState {
    /// PowerShell and the Defender module start in a second or two; `doctor` must not wait longer
    /// than a few seconds for a stuck Defender service.
    pub const TIMEOUT: Duration = Duration::from_secs(5);
}

impl Default for PowerShellDefenderState {
    fn default() -> Self {
        Self { timeout: Self::TIMEOUT }
    }
}

#[async_trait]
impl DefenderStateReader for PowerShellDefenderState {
    async fn state(&self) -> PortResult<DefenderState> {
        let mut command = system_powershell(STATE_SCRIPT).ok_or_else(|| PortError::process(PROGRAM, "the Windows system folder is unknown"))?;
        let timeout = self.timeout;
        let code = tokio::task::spawn_blocking(move || run_bounded(&mut command, timeout)).await.map_err(|e| PortError::process(PROGRAM, e))?;
        match code {
            Some(code) => state_of(code),
            None => Err(PortError::process(PROGRAM, format!("Get-MpComputerStatus could not be run or did not answer within {} s", timeout.as_secs()))),
        }
    }
}

fn state_of(code: i32) -> PortResult<DefenderState> {
    match code {
        REAL_TIME_PROTECTION => Ok(DefenderState::RealTimeProtection),
        UNAVAILABLE => Ok(DefenderState::Unavailable),
        PASSIVE => Ok(DefenderState::Passive),
        REAL_TIME_PROTECTION_OFF => Ok(DefenderState::RealTimeProtectionOff),
        other => Err(PortError::process(PROGRAM, format!("Get-MpComputerStatus failed (exit code {other})"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_map_to_states() {
        assert_eq!(state_of(REAL_TIME_PROTECTION), Ok(DefenderState::RealTimeProtection));
        assert_eq!(state_of(UNAVAILABLE), Ok(DefenderState::Unavailable));
        assert_eq!(state_of(PASSIVE), Ok(DefenderState::Passive));
        assert_eq!(state_of(REAL_TIME_PROTECTION_OFF), Ok(DefenderState::RealTimeProtectionOff));
        assert!(matches!(state_of(1), Err(PortError::Process { .. })), "a PowerShell error outside the script's checks is unknown");
    }

    #[test]
    fn the_script_is_a_constant_reading_only_the_status() {
        for code in [REAL_TIME_PROTECTION, UNAVAILABLE, PASSIVE, REAL_TIME_PROTECTION_OFF] {
            assert!(STATE_SCRIPT.contains(&format!("exit {code}")), "uses exit {code}");
        }
        assert!(!STATE_SCRIPT.contains('"') && !STATE_SCRIPT.contains("$env:"), "no interpolation, no input");
        assert!(!STATE_SCRIPT.contains("MpPreference"), "never the exclusions (they need elevation)");
    }

    #[cfg(windows)]
    #[test]
    fn the_script_parses() {
        assert!(super::super::powershell::script_parses(STATE_SCRIPT));
    }

    /// Whatever this machine's Defender does, reading it is never an error that blocks: it ends
    /// within the timeout with a state or a `Process` error.
    #[cfg(windows)]
    #[tokio::test]
    async fn reading_the_state_ends_within_the_timeout() {
        let started = std::time::Instant::now();
        let state = PowerShellDefenderState::default().state().await;
        assert!(state.is_ok() || matches!(state, Err(PortError::Process { .. })), "{state:?}");
        assert!(started.elapsed() < PowerShellDefenderState::TIMEOUT + Duration::from_secs(1));
    }
}
