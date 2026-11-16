//! `services`: the services the plan needs are running. A process outside Lampo (an
//! uninstaller, an antivirus) may have killed them: `lampo status` shows it, and so must the
//! doctor.

use super::{DoctorCheck, DoctorFacts, check};
use crate::dto::{CheckStatus, DoctorCheckDto, DoctorReportDto, ServiceStateDto, ServiceStatusDto};

const ID: &str = "services";
/// Starts every service of the plan (and the app, when it is not running).
const START_ALL: &str = "lampo up";

/// Reads [`DoctorFacts::services`]. Needed = in the plan: used by an active site, or the
/// always-on MySQL. `Error` when a needed service is `failed`, `Warning` when
/// one is stopped or its state cannot be read. Services nobody needs are not findings.
pub struct ServicesCheck;

impl DoctorCheck for ServicesCheck {
    fn run(&self, facts: &DoctorFacts, report: &mut DoctorReportDto) -> DoctorCheckDto {
        let statuses = match &facts.services {
            Ok(statuses) => statuses,
            Err(e) => return check(ID, CheckStatus::Warning, format!("the service states cannot be read: {}", e.message), Some(START_ALL.into())),
        };
        let needed: Vec<&ServiceStatusDto> = statuses.iter().filter(|s| s.required).collect();
        let failed = in_state(&needed, ServiceStateDto::Failed);
        let stopped = in_state(&needed, ServiceStateDto::Stopped);
        let unknown = in_state(&needed, ServiceStateDto::Unknown);
        report.services_down = failed.iter().chain(&stopped).map(|s| s.service.clone()).collect();

        let status = match (failed.is_empty(), stopped.is_empty() && unknown.is_empty()) {
            (true, true) => return check(ID, CheckStatus::Ok, "every service Lampo should run is running", None),
            (false, _) => CheckStatus::Error,
            (true, false) => CheckStatus::Warning,
        };
        let findings: Vec<String> = [("failed", &failed), ("stopped", &stopped), ("state unknown", &unknown)]
            .into_iter()
            .filter(|(_, services)| !services.is_empty())
            .map(|(what, services)| format!("{what}: {}", services.iter().map(|s| describe(s)).collect::<Vec<_>>().join(", ")))
            .collect();
        check(ID, status, format!("services Lampo should run are not running ({})", findings.join("; ")), Some(fix(&report.services_down)))
    }
}

fn in_state<'a>(services: &[&'a ServiceStatusDto], state: ServiceStateDto) -> Vec<&'a ServiceStatusDto> {
    services.iter().copied().filter(|s| s.state == state).collect()
}

/// `apache` or, with the runner's explanation, `apache (exited with code 1)`.
fn describe(service: &ServiceStatusDto) -> String {
    match service.message.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        Some(message) => format!("{} ({message})", service.service),
        None => service.service.clone(),
    }
}

/// `lampo up apache mysql-8.4`: starts just those (a failed one is started again); with only
/// unknown states, the whole plan.
fn fix(down: &[String]) -> String {
    match down.is_empty() {
        true => START_ALL.into(),
        false => format!("{START_ALL} {}", down.join(" ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apache(message: Option<&str>) -> ServiceStatusDto {
        ServiceStatusDto {
            service: "apache".into(),
            kind: "apache".into(),
            label: "Apache".into(),
            state: ServiceStateDto::Failed,
            pid: None,
            port: Some(80),
            ui_port: None,
            ui_url: None,
            message: message.map(Into::into),
            required: true,
            required_by: vec!["shop".into()],
        }
    }

    #[test]
    fn a_service_is_described_with_the_runner_explanation() {
        assert_eq!(describe(&apache(Some("exited with code 1"))), "apache (exited with code 1)");
        assert_eq!(describe(&apache(Some("  "))), "apache");
        assert_eq!(describe(&apache(None)), "apache");
    }

    #[test]
    fn the_fix_starts_just_the_services_down() {
        assert_eq!(fix(&["apache".into(), "mysql-8.4".into()]), "lampo up apache mysql-8.4");
        assert_eq!(fix(&[]), "lampo up");
    }
}
