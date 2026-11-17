//! `defender`: Microsoft Defender's real-time protection scans every PHP source the first request
//! to a site opens (8-24 s after each definition update, reboot or change), unless the PHP
//! processes are excluded. A standard user cannot read Defender's exclusions, so the check relies
//! on what Lampo remembers of its own requests ([`ProcessExclusionRecord`]): a `Warning` when
//! Defender scans on access and an installed `php-cgi.exe` is not among the executables Defender
//! confirmed excluded. Never an `Error`, and never a `Warning` when Defender is not scanning or
//! its state cannot be read.

use std::path::Path;
use std::sync::Arc;

use lampo_domain::ports::ProcessExclusionRecordReader;
use lampo_domain::scan_exclusion::{DefenderState, ProcessExclusionRecord, ScanExclusionOutcome};

use super::super::scan_exclusion::{ADD_PHP_COMMAND, php_servers};
use super::{DoctorCheck, DoctorFacts, check};
use crate::dto::{CheckStatus, DoctorCheckDto, DoctorReportDto};

const ID: &str = "defender";
/// The commands for whoever manages a PC where a policy controls Defender.
const IT_COMMANDS: &str = "lampo defender exclusion commands --php";

/// Reads [`DoctorFacts::defender`] and [`DoctorFacts::installed`], and the record of the requests.
pub struct DefenderCheck {
    pub record: Arc<dyn ProcessExclusionRecordReader>,
}

impl DefenderCheck {
    /// A record that does not load counts as no request made: the advice is then a new one.
    fn record(&self) -> ProcessExclusionRecord {
        self.record.read().unwrap_or_else(|e| {
            tracing::warn!("the record of the Defender exclusions does not load: {e}");
            ProcessExclusionRecord::default()
        })
    }
}

impl DoctorCheck for DefenderCheck {
    fn run(&self, facts: &DoctorFacts, _report: &mut DoctorReportDto) -> DoctorCheckDto {
        let Some(state) = facts.defender else {
            return check(ID, CheckStatus::Ok, "Microsoft Defender's state cannot be read: if the first request to a site is slow, exclude the PHP processes from its scan", None);
        };
        match state {
            DefenderState::RealTimeProtection => {}
            DefenderState::RealTimeProtectionOff => return check(ID, CheckStatus::Ok, "Microsoft Defender's real-time protection is off: it does not slow PHP down", None),
            DefenderState::Passive => return check(ID, CheckStatus::Ok, "Microsoft Defender runs in passive mode next to another antivirus: it does not slow PHP down", None),
            DefenderState::Unavailable => return check(ID, CheckStatus::Ok, "Microsoft Defender is not the active antivirus: it does not slow PHP down", None),
        }
        let php_servers = php_servers(&facts.installed);
        if php_servers.is_empty() {
            return check(ID, CheckStatus::Ok, "no PHP is installed for Microsoft Defender to slow down", None);
        }
        let record = self.record();
        let uncovered: Vec<&Path> = php_servers.iter().copied().filter(|exe| !record.is_excluded(exe)).collect();
        if uncovered.is_empty() {
            return check(ID, CheckStatus::Ok, "Microsoft Defender's real-time protection skips the files every installed PHP opens (process exclusion applied by Lampo)", None);
        }
        let finding = match uncovered.len() == php_servers.len() && record.excluded.is_empty() {
            true => "the PHP processes are not excluded from its scan".to_string(),
            false => format!("not covered by the exclusion Lampo applied: {}", uncovered.iter().map(|exe| exe.display().to_string()).collect::<Vec<_>>().join(", ")),
        };
        check(
            ID,
            CheckStatus::Warning,
            format!(
                "Microsoft Defender's real-time protection scans every PHP file the first request to a site opens (8-24 s): {finding}{}",
                last_outcome(&record).map(|outcome| format!(" ({outcome})")).unwrap_or_default()
            ),
            Some(format!("{ADD_PHP_COMMAND} (where a policy manages Defender, give the commands of `{IT_COMMANDS}` to whoever manages the PC)")),
        )
    }
}

/// Why the last request did not exclude anything, when it did not.
fn last_outcome(record: &ProcessExclusionRecord) -> Option<&'static str> {
    match record.last_request.as_ref()?.outcome {
        ScanExclusionOutcome::Applied => None,
        ScanExclusionOutcome::Cancelled => Some("the last request was cancelled at the UAC prompt"),
        ScanExclusionOutcome::DefenderUnavailable => Some("at the last request Defender was not the active antivirus"),
        ScanExclusionOutcome::Refused => Some("Defender refused the last request: Tamper Protection or a policy controls its exclusions"),
        ScanExclusionOutcome::NotConfirmed => Some("the last request was not confirmed: a policy probably manages Defender's exclusions"),
    }
}
