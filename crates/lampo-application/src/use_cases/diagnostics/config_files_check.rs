//! The configuration files that do not load: `settings.toml` (nothing can be synced without it)
//! and the project files (their projects are hidden from every listing until repaired).

use super::{DoctorCheck, DoctorFacts, check};
use crate::dto::{CheckStatus, DoctorCheckDto, DoctorReportDto};
use crate::mapping::unreadable_project_to_dto;

const SETTINGS_ID: &str = "settings";
const PROJECT_FILES_ID: &str = "project-files";

/// `settings`: `settings.toml` loads ([`DoctorFacts::settings_problem`]). `Error` otherwise: the
/// app runs on nothing else, and the other checks fell back to the default settings.
pub struct SettingsFileCheck;

impl DoctorCheck for SettingsFileCheck {
    fn run(&self, facts: &DoctorFacts, _report: &mut DoctorReportDto) -> DoctorCheckDto {
        match &facts.settings_problem {
            None => check(SETTINGS_ID, CheckStatus::Ok, "the settings load", None),
            Some(reason) => check(
                SETTINGS_ID,
                CheckStatus::Error,
                format!("the settings do not load ({reason}); the other checks used the default settings"),
                Some("repair settings.toml by hand (deleting it restores the default settings)".into()),
            ),
        }
    }
}

/// `project-files`: every project file loads. `Warning` otherwise, listing each file with its
/// reason in `unreadable_projects`: the project is hidden, its ports and id stay reserved.
pub struct ProjectFilesCheck;

impl DoctorCheck for ProjectFilesCheck {
    fn run(&self, facts: &DoctorFacts, report: &mut DoctorReportDto) -> DoctorCheckDto {
        report.unreadable_projects = facts.project_files.unreadable.iter().map(unreadable_project_to_dto).collect();
        match report.unreadable_projects.is_empty() {
            true => check(PROJECT_FILES_ID, CheckStatus::Ok, "every project file loads", None),
            false => check(
                PROJECT_FILES_ID,
                CheckStatus::Warning,
                format!("project files that do not load (their projects are hidden): {}", report.unreadable_projects.iter().map(|u| u.reason.as_str()).collect::<Vec<_>>().join("; ")),
                Some("repair the file by hand, or delete it to remove the project".into()),
            ),
        }
    }
}
