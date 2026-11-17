//! What to propose about the PHP process exclusion after a PHP is installed or uninstalled
//! (`lampo package install|uninstall`, the GUI's packages): a process exclusion covers only the
//! executables installed when it was requested, so a new PHP is scanned again and an uninstalled
//! one leaves an exclusion of a file that no longer exists.
//!
//! Only proposed, never made: each change is an explicit request of the user (one UAC prompt,
//! [`super::scan_exclusion::DefenderExclusion::change`]). Only when Lampo already applied the
//! exclusion (the record lists an executable Defender confirmed excluded): a user who never asked
//! is not insisted on here, the doctor's `defender` check tells them. And only when Microsoft
//! Defender is the active antivirus: adding needs its real-time protection on (otherwise nothing
//! is slowed down), removing needs it running (otherwise the request cannot be made).
//!
//! Advice, not part of the package change: whatever cannot be read (the record, Defender's
//! state, the installed packages) is logged and leaves nothing to propose.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lampo_domain::ports::{DefenderStateReader, InstalledPackageRegistry, ProcessExclusionRecordReader};
use lampo_domain::scan_exclusion::{DefenderState, ProcessExclusionRecord};

use super::scan_exclusion::{ADD_PHP_COMMAND, REMOVE_UNINSTALLED_COMMAND, php_servers, server_executables};
use crate::dto::{DefenderExclusionRequestDto, DefenderExclusionSuggestionDto, ScanExclusionChangeDto};

pub struct DefenderExclusionAdvice {
    defender: Arc<dyn DefenderStateReader>,
    record: Arc<dyn ProcessExclusionRecordReader>,
    installed: Arc<dyn InstalledPackageRegistry>,
}

/// The executables a change would concern, before Defender's state is known.
struct Pending {
    /// Installed `php-cgi.exe` not excluded.
    uncovered: Vec<PathBuf>,
    /// Excluded executables no longer installed.
    uninstalled: Vec<PathBuf>,
}

impl DefenderExclusionAdvice {
    /// `record`: what Lampo remembers of its requests naming the processes.
    pub fn new(defender: Arc<dyn DefenderStateReader>, record: Arc<dyn ProcessExclusionRecordReader>, installed: Arc<dyn InstalledPackageRegistry>) -> Self {
        Self { defender, record, installed }
    }

    /// The changes to propose now, adding first: none when the exclusion was never applied,
    /// already covers what is installed, or Defender does not need it. Defender's state (a few
    /// seconds at most) is read only when there is something to propose.
    pub async fn suggestions(&self) -> Vec<DefenderExclusionSuggestionDto> {
        let Some(pending) = self.pending() else { return Vec::new() };
        if pending.uncovered.is_empty() && pending.uninstalled.is_empty() {
            return Vec::new();
        }
        let state = match self.defender.state().await {
            Ok(state) => state,
            Err(e) => {
                tracing::warn!("cannot read Microsoft Defender's state, no exclusion change proposed: {e}");
                return Vec::new();
            }
        };
        let add = (state == DefenderState::RealTimeProtection && !pending.uncovered.is_empty())
            .then(|| suggestion(ScanExclusionChangeDto::Add, &pending.uncovered, DefenderExclusionRequestDto::new(ScanExclusionChangeDto::Add, true, false), ADD_PHP_COMMAND));
        let remove = (is_active(state) && !pending.uninstalled.is_empty())
            .then(|| suggestion(ScanExclusionChangeDto::Remove, &pending.uninstalled, DefenderExclusionRequestDto::remove_uninstalled(), REMOVE_UNINSTALLED_COMMAND));
        add.into_iter().chain(remove).collect()
    }

    /// `None` when there is nothing to compare: no exclusion applied, or a read failing.
    fn pending(&self) -> Option<Pending> {
        let record = self.record.read().inspect_err(|e| tracing::warn!("the record of the Defender exclusions does not load: {e}")).ok()?;
        if record.excluded.is_empty() {
            return None;
        }
        let installed = self.installed.list().inspect_err(|e| tracing::warn!("cannot list the installed packages for the Defender exclusion: {e}")).ok()?;
        Some(Pending { uncovered: uncovered(&record, &php_servers(&installed)), uninstalled: record.excluded_except(&server_executables(&installed)) })
    }
}

fn uncovered(record: &ProcessExclusionRecord, php_servers: &[&Path]) -> Vec<PathBuf> {
    php_servers.iter().filter(|exe| !record.is_excluded(exe)).map(|exe| exe.to_path_buf()).collect()
}

/// Defender is the active antivirus, its real-time protection on or not.
fn is_active(state: DefenderState) -> bool {
    matches!(state, DefenderState::RealTimeProtection | DefenderState::RealTimeProtectionOff)
}

fn suggestion(change: ScanExclusionChangeDto, processes: &[PathBuf], request: DefenderExclusionRequestDto, command: &str) -> DefenderExclusionSuggestionDto {
    DefenderExclusionSuggestionDto { change, processes: processes.iter().map(|exe| exe.display().to_string()).collect(), request, command: command.to_string() }
}
