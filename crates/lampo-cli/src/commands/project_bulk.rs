//! `project activate --all` / `project deactivate --all`: the app's «Attiva tutti» / «Disattiva
//! tutti» (`apps/desktop/src/lib/projects/bulkActivation.ts`), through the same control request
//! as one project at a time.

use lampo_application::control::ControlRequest;
use lampo_application::dto::{ProjectDto, ProjectListDto, SetProjectActiveRequest};
use lampo_application::{AppError, AppResult, ErrorCode};
use serde::Serialize;

use super::Cmd;
use crate::error_doc;
use crate::exit_code::ExitCode;

/// Failures that are about reaching the app, not about one project: every other project would
/// fail the same way, so the command stops with the error instead.
const FATAL_CODES: [ErrorCode; 2] = [ErrorCode::AppUnavailable, ErrorCode::ProtocolMismatch];

/// `--json` result of `project activate --all` / `project deactivate --all`.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BulkActivationDto {
    /// The state asked for.
    pub active: bool,
    /// Projects switched, including those whose hosts file still waits for the administrator.
    pub changed: Vec<String>,
    /// Projects left as they were because their folder is missing (they cannot be served).
    pub missing_folder: Vec<String>,
    /// Projects the app refused, left as they were.
    pub failed: Vec<BulkActivationFailureDto>,
    /// At least one change still has to reach the hosts file (`--allow-elevation`).
    pub elevation_pending: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BulkActivationFailureDto {
    pub project_id: String,
    pub code: ErrorCode,
    pub message: String,
}

/// Switches every project not already in the `active` state, one at a time (each change rewrites
/// Apache's configuration: running them together would only race). A refusal does not stop the
/// others; the exit code is the first refusal's, else 7 when the hosts file still waits.
pub async fn set_all_active(c: &Cmd<'_>, active: bool) -> AppResult<i32> {
    let list = c.ctx.projects.list()?;
    let (targets, missing_folder) = targets(&list.projects, active);
    let mut result = BulkActivationDto { active, changed: Vec::new(), missing_folder, failed: Vec::new(), elevation_pending: false };
    for project_id in targets {
        let request = ControlRequest::ProjectSetActive(SetProjectActiveRequest { project_id: project_id.clone(), active });
        match c.app::<ProjectDto>(request).await {
            Ok(_) => result.changed.push(project_id),
            Err(e) if e.code == ErrorCode::ElevationRequired => {
                result.changed.push(project_id);
                result.elevation_pending = true;
            }
            Err(e) if FATAL_CODES.contains(&e.code) => return Err(e),
            Err(AppError { code, message, .. }) => result.failed.push(BulkActivationFailureDto { project_id, code, message }),
        }
    }
    c.io.emit(&result, render);
    if let Some(warning) = unreadable_warning(&list) {
        c.io.err_line(&warning);
    }
    if result.elevation_pending {
        c.io.err_line(&format!("lampo: warning: the hosts file was not updated. {}", error_doc::hint_for(ErrorCode::ElevationRequired)));
    }
    Ok(exit_code(&result).code())
}

/// The ids to switch (not already `active`, folder present) and the ones skipped because their
/// folder is missing.
fn targets(projects: &[ProjectDto], active: bool) -> (Vec<String>, Vec<String>) {
    let (ready, missing): (Vec<&ProjectDto>, Vec<&ProjectDto>) = projects.iter().filter(|p| p.active != active).partition(|p| p.folder_exists);
    let ids = |list: Vec<&ProjectDto>| list.into_iter().map(|p| p.id.clone()).collect();
    (ids(ready), ids(missing))
}

fn exit_code(result: &BulkActivationDto) -> ExitCode {
    match result.failed.first() {
        Some(failure) => failure.code.into(),
        None if result.elevation_pending => ExitCode::ElevationRequired,
        None => ExitCode::Success,
    }
}

/// The project files that do not load are never switched: say so on stderr.
fn unreadable_warning(list: &ProjectListDto) -> Option<String> {
    if list.unreadable.is_empty() {
        return None;
    }
    let reasons: Vec<&str> = list.unreadable.iter().map(|u| u.reason.as_str()).collect();
    Some(format!("lampo: warning: {} project file(s) do not load and were left as they are: {} (see `lampo doctor`)", list.unreadable.len(), reasons.join("; ")))
}

fn render(r: &BulkActivationDto) -> String {
    let verb = if r.active { "activated" } else { "deactivated" };
    let mut lines = Vec::new();
    if r.changed.is_empty() && r.failed.is_empty() && r.missing_folder.is_empty() {
        lines.push(if r.active { "every project is already active".to_string() } else { "no project is active".to_string() });
    }
    if !r.changed.is_empty() {
        lines.push(format!("{verb} {} project(s): {}", r.changed.len(), r.changed.join(", ")));
    }
    if !r.missing_folder.is_empty() {
        lines.push(format!("skipped, folder missing: {}", r.missing_folder.join(", ")));
    }
    lines.extend(r.failed.iter().map(|f| format!("not {verb}: {}: {}", f.project_id, f.message)));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dto(changed: &[&str], missing: &[&str], failed: &[(&str, ErrorCode)], elevation_pending: bool) -> BulkActivationDto {
        BulkActivationDto {
            active: true,
            changed: changed.iter().map(|s| s.to_string()).collect(),
            missing_folder: missing.iter().map(|s| s.to_string()).collect(),
            failed: failed.iter().map(|(id, code)| BulkActivationFailureDto { project_id: id.to_string(), code: *code, message: "boom".into() }).collect(),
            elevation_pending,
        }
    }

    #[test]
    fn the_first_refusal_sets_the_exit_code_then_the_pending_hosts_file() {
        assert_eq!(exit_code(&dto(&["a"], &[], &[], false)), ExitCode::Success);
        assert_eq!(exit_code(&dto(&["a"], &["gone"], &[], false)), ExitCode::Success, "a missing folder is skipped, not refused");
        assert_eq!(exit_code(&dto(&["a"], &[], &[], true)), ExitCode::ElevationRequired);
        assert_eq!(exit_code(&dto(&[], &[], &[("b", ErrorCode::PortInUse), ("c", ErrorCode::NotFound)], true)), ExitCode::Conflict);
    }

    #[test]
    fn renders_what_changed_what_was_skipped_and_what_was_refused() {
        assert_eq!(render(&dto(&["a", "b"], &["gone"], &[("c", ErrorCode::PortInUse)], false)), "activated 2 project(s): a, b\nskipped, folder missing: gone\nnot activated: c: boom");
        assert_eq!(render(&dto(&[], &[], &[], false)), "every project is already active");
        assert_eq!(render(&BulkActivationDto { active: false, ..dto(&[], &[], &[], false) }), "no project is active");
    }
}
