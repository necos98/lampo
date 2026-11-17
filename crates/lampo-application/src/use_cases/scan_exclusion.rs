//! The optional Microsoft Defender exclusions (`lampo defender exclusion add|remove|commands --php
//! --mysql --projects`, `lampo mysql defender-exclusion`, Settings):
//!
//! * `mysql`: the folder holding every MySQL instance's data (`<data root>\mysql`);
//! * `php`: one process exclusion per server executable Lampo runs from its packages:
//!   `php-cgi.exe` of every installed PHP and `httpd.exe` of every installed Apache. Defender then
//!   skips the files those processes open (every project's PHP sources, wherever they are)
//!   without excluding any source folder. `mysqld.exe` is not named: the data folder already
//!   covers what it writes, and `mysql` must stay what it always was;
//! * `projects`: one folder holding the user's projects, named by the user or suggested: the
//!   deepest folder holding the parent folders of every registered project that the adapter
//!   accepts ([`ScanExclusionEditor::accepted_projects_folder`]). The suggestion only spares
//!   typing: the projects' configuration is a file the user can write, so the adapter and the
//!   elevated helper check the folder themselves.
//!
//! Only on an explicit request: nothing adds an exclusion at install or start. Each request shows
//! one UAC prompt whatever it names, so a call that may not prompt (the CLI without
//! `--allow-elevation`) is refused before anything happens. The executables are those installed
//! when the request is made: a PHP installed later needs a new request. The outcome of every
//! request naming `php` is remembered ([`ProcessExclusionRecord`]): the doctor's `defender` check
//! tells from it whether every installed PHP is covered.
//!
//! When a policy controls Defender's exclusions (group policy, Intune), `commands` gives the
//! `Add-MpPreference` lines of the same targets, for whoever manages the PC; it prompts for
//! nothing and changes nothing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lampo_domain::package::{EntryRole, PackageKind};
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{InstalledPackageRegistry, ProcessExclusionRecordReader, ProcessExclusionRecordWriter, ProjectRepository, ScanExclusionEditor};
use lampo_domain::project::ProjectRoot;
use lampo_domain::scan_exclusion::{ProcessExclusionRecord, ProcessExclusionRequest, ScanExclusionChange, ScanExclusionOutcome, ScanExclusionTargets};
use lampo_domain::service::ServiceKind;

use super::defender_commands;
use super::support::validation;
use crate::context::{CallContext, ElevationPolicy};
use crate::dto::{
    DefenderExclusionCommandDto, DefenderExclusionInfoDto, DefenderExclusionRequestDto, DefenderExclusionResultDto, ScanExclusionChangeDto, ScanExclusionOutcomeDto, ScanExclusionResultDto,
};
use crate::error::{AppError, AppResult, ErrorCode};

/// The server executable of PHP: the one opening the projects' sources.
pub(crate) const PHP_SERVER: (PackageKind, EntryRole) = (PackageKind::PHP, EntryRole::FastCgi);
/// The server executable of each package kind the `php` part names.
const SERVER_EXECUTABLES: [(PackageKind, EntryRole); 2] = [PHP_SERVER, (PackageKind::APACHE, EntryRole::Server)];
const SEPARATOR: char = '\\';

pub struct DefenderExclusion {
    editor: Arc<dyn ScanExclusionEditor>,
    installed: Arc<dyn InstalledPackageRegistry>,
    projects: Arc<dyn ProjectRepository>,
    paths: LampoPaths,
    record_reader: Arc<dyn ProcessExclusionRecordReader>,
    record_writer: Arc<dyn ProcessExclusionRecordWriter>,
    /// One prompt at a time.
    busy: tokio::sync::Mutex<()>,
}

impl DefenderExclusion {
    /// `record_reader` and `record_writer`: the record of the requests naming `php` (one adapter).
    pub fn new(
        editor: Arc<dyn ScanExclusionEditor>,
        installed: Arc<dyn InstalledPackageRegistry>,
        projects: Arc<dyn ProjectRepository>,
        paths: LampoPaths,
        record_reader: Arc<dyn ProcessExclusionRecordReader>,
        record_writer: Arc<dyn ProcessExclusionRecordWriter>,
    ) -> Self {
        Self { editor, installed, projects, paths, record_reader, record_writer, busy: tokio::sync::Mutex::new(()) }
    }

    fn folder(&self) -> PathBuf {
        self.paths.service_kind_dir(&ServiceKind::MYSQL)
    }

    /// `php-cgi.exe` of every installed PHP and `httpd.exe` of every installed Apache.
    fn server_executables(&self) -> AppResult<Vec<PathBuf>> {
        let mut executables: Vec<PathBuf> = self
            .installed
            .list()?
            .iter()
            .filter_map(|package| SERVER_EXECUTABLES.iter().find(|(kind, _)| *kind == package.id.kind).and_then(|(_, role)| package.entry(*role)))
            .map(Path::to_path_buf)
            .collect();
        executables.sort();
        executables.dedup();
        Ok(executables)
    }

    /// The deepest folder holding every registered project's parent folder that the adapter
    /// accepts, if any.
    fn suggested_projects_folder(&self) -> AppResult<Option<PathBuf>> {
        let roots: Vec<ProjectRoot> = self.projects.list()?.into_iter().map(|project| project.root).collect();
        // A refusal is the adapter's answer, not a failure: that folder is just not suggested.
        Ok(projects_folder_candidates(&roots).into_iter().find_map(|candidate| self.editor.accepted_projects_folder(&candidate).ok()))
    }

    /// The folder a `projects` request names: `requested` (normalised) or the suggestion.
    fn projects_folder(&self, requested: Option<&str>) -> AppResult<PathBuf> {
        match requested {
            Some(folder) => Ok(PathBuf::from(ProjectRoot::new(folder)?.as_str())),
            None => self.suggested_projects_folder()?.ok_or_else(|| {
                validation(
                    "no folder holding your projects can be excluded (it must be inside your user profile, not Desktop, Documents or Downloads themselves): \
                     name one with --projects <folder>",
                )
            }),
        }
    }

    /// What `mysql`, `php` and `projects` would cover now.
    pub fn info(&self) -> AppResult<DefenderExclusionInfoDto> {
        Ok(DefenderExclusionInfoDto {
            folder: display(&self.folder()),
            processes: self.server_executables()?.iter().map(|p| display(p)).collect(),
            projects_folder: self.suggested_projects_folder()?.as_deref().map(display),
        })
    }

    /// Asks Defender to add or remove the exclusions `request` names, in one UAC prompt.
    /// `Validation` when it names nothing, a projects folder without `projects` or one that
    /// cannot be excluded (or none can be suggested), `ElevationRequired` when `context` may not
    /// prompt, `PackageMissing` for `php` without any PHP or Apache installed; the outcomes the
    /// user can act on are results, not errors.
    pub async fn change(&self, request: DefenderExclusionRequestDto, context: CallContext) -> AppResult<DefenderExclusionResultDto> {
        check_request(&request)?;
        if context.elevation == ElevationPolicy::Deny {
            return Err(AppError::new(ErrorCode::ElevationRequired, "changing the Microsoft Defender exclusions shows a UAC prompt: re-run with --allow-elevation"));
        }
        let _busy = self.busy.lock().await;
        let targets = self.targets(&request)?;
        let change = change_from_dto(request.change);
        let outcome = self.editor.change(&targets, change).await?;
        if request.php {
            self.remember(ProcessExclusionRequest { change, outcome, processes: targets.processes.clone() });
        }
        Ok(DefenderExclusionResultDto {
            change: request.change,
            outcome: outcome_to_dto(outcome),
            folder: targets.folder.as_deref().map(display),
            processes: targets.processes.iter().map(|p| display(p)).collect(),
            projects: targets.projects.as_deref().map(display),
        })
    }

    /// The PowerShell commands making the change `request` names (`lampo defender exclusion
    /// commands`, Settings), for whoever manages a PC where Defender refuses Lampo's request: the
    /// targets [`Self::change`] would send, after the same checks and with the same errors, but
    /// with neither a prompt nor any change to the system.
    pub fn commands(&self, request: DefenderExclusionRequestDto) -> AppResult<Vec<DefenderExclusionCommandDto>> {
        check_request(&request)?;
        let change = change_from_dto(request.change);
        let targets = self.editor.accepted_targets(&self.targets(&request)?, change)?;
        Ok(defender_commands::exclusion_commands(&targets, change))
    }

    /// The targets `request` names now. `PackageMissing` for `php` without any PHP or Apache.
    fn targets(&self, request: &DefenderExclusionRequestDto) -> AppResult<ScanExclusionTargets> {
        let processes = if request.php { self.server_executables()? } else { Vec::new() };
        if request.php && processes.is_empty() {
            return Err(AppError::new(ErrorCode::PackageMissing, "no PHP or Apache package is installed: there is no php-cgi.exe or httpd.exe to exclude"));
        }
        let projects = if request.projects { Some(self.projects_folder(request.projects_folder.as_deref())?) } else { None };
        Ok(ScanExclusionTargets { folder: request.mysql.then(|| self.folder()), processes, projects })
    }

    /// Adds `request` to the record. The request itself is done: a record that cannot be read
    /// starts over, one that cannot be written is logged (the doctor then advises a new request).
    fn remember(&self, request: ProcessExclusionRequest) {
        let mut record = self.record_reader.read().unwrap_or_else(|e| {
            tracing::warn!("the record of the Defender exclusions does not load, starting a new one: {e}");
            ProcessExclusionRecord::default()
        });
        record.record(request);
        if let Err(e) = self.record_writer.write(&record) {
            tracing::warn!("cannot record the outcome of the Defender exclusion request: {e}");
        }
    }

    /// The MySQL-only form of [`Self::change`].
    pub async fn change_mysql(&self, change: ScanExclusionChangeDto, context: CallContext) -> AppResult<ScanExclusionResultDto> {
        let result = self.change(DefenderExclusionRequestDto::new(change, false, true), context).await?;
        Ok(ScanExclusionResultDto { change, outcome: result.outcome, folder: display(&self.folder()) })
    }
}

/// `Validation` when `request` names nothing, or a projects folder without `projects`.
fn check_request(request: &DefenderExclusionRequestDto) -> AppResult<()> {
    if !request.php && !request.mysql && !request.projects {
        return Err(validation("name what to exclude: --php, --mysql, --projects or more of them"));
    }
    if request.projects_folder.is_some() && !request.projects {
        return Err(validation("a projects folder needs `projects`"));
    }
    Ok(())
}

/// The folders that may hold every project, deepest first: the common ancestor of the projects'
/// parent folders (ignoring case, spelled as the first project's), then its ancestors down to
/// the first folder below the drive root. Projects on a network share are left out; none without
/// projects or when they are on different drives.
fn projects_folder_candidates(roots: &[ProjectRoot]) -> Vec<PathBuf> {
    let parents: Vec<Vec<&str>> = roots
        .iter()
        .map(|root| root.as_str().split(SEPARATOR).filter(|part| !part.is_empty()).collect::<Vec<_>>())
        .filter(|parts| parts.first().is_some_and(|drive| drive.ends_with(':')))
        .map(|mut parts| {
            parts.pop();
            parts
        })
        .collect();
    let Some((first, others)) = parents.split_first() else { return Vec::new() };
    let common = others.iter().fold(first.len(), |depth, parts| first.iter().zip(parts).take(depth).take_while(|(a, b)| a.to_lowercase() == b.to_lowercase()).count());
    (2..=common).rev().map(|depth| PathBuf::from(first[..depth].join("\\"))).collect()
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn change_from_dto(change: ScanExclusionChangeDto) -> ScanExclusionChange {
    match change {
        ScanExclusionChangeDto::Add => ScanExclusionChange::Add,
        ScanExclusionChangeDto::Remove => ScanExclusionChange::Remove,
    }
}

fn outcome_to_dto(outcome: ScanExclusionOutcome) -> ScanExclusionOutcomeDto {
    match outcome {
        ScanExclusionOutcome::Applied => ScanExclusionOutcomeDto::Applied,
        ScanExclusionOutcome::Cancelled => ScanExclusionOutcomeDto::Cancelled,
        ScanExclusionOutcome::DefenderUnavailable => ScanExclusionOutcomeDto::DefenderUnavailable,
        ScanExclusionOutcome::Refused => ScanExclusionOutcomeDto::Refused,
        ScanExclusionOutcome::NotConfirmed => ScanExclusionOutcomeDto::NotConfirmed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(roots: &[&str]) -> Vec<String> {
        let roots: Vec<ProjectRoot> = roots.iter().map(|root| ProjectRoot::new(root).unwrap()).collect();
        projects_folder_candidates(&roots).iter().map(|p| display(p)).collect()
    }

    #[test]
    fn candidates_start_from_the_common_parent_folder() {
        assert_eq!(candidates(&[r"C:\Users\me\Desktop\Progetti\shop"]), [r"C:\Users\me\Desktop\Progetti", r"C:\Users\me\Desktop", r"C:\Users\me", r"C:\Users"]);
        assert_eq!(
            candidates(&[r"C:\Users\me\Progetti\a", r"c:\users\ME\progetti\clienti\b"]),
            [r"C:\Users\me\Progetti", r"C:\Users\me", r"C:\Users"],
            "ignoring case, the first one's spelling"
        );
        assert_eq!(candidates(&[r"C:\Users\me\Desktop\Progetti\a", r"C:\Users\me\Documents\b"]), [r"C:\Users\me", r"C:\Users"]);
    }

    #[test]
    fn no_candidate_without_a_common_folder_below_a_drive() {
        assert!(candidates(&[]).is_empty());
        assert!(candidates(&[r"C:\Users\me\a", r"D:\Users\me\b"]).is_empty(), "two drives");
        assert!(candidates(&[r"C:\site"]).is_empty(), "the drive root");
        assert!(candidates(&[r"\\server\share\site"]).is_empty(), "a network share");
        assert_eq!(candidates(&[r"\\server\share\site", r"C:\Users\me\Progetti\a"]), [r"C:\Users\me\Progetti", r"C:\Users\me", r"C:\Users"], "shares are left out");
    }
}
