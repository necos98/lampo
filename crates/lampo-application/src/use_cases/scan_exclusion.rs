//! The optional Microsoft Defender exclusions (`lampo defender exclusion add|remove --php
//! --mysql`, `lampo mysql defender-exclusion`, Settings):
//!
//! * `mysql`: the folder holding every MySQL instance's data (`<data root>\mysql`);
//! * `php`: one process exclusion per server executable Lampo runs from its packages:
//!   `php-cgi.exe` of every installed PHP and `httpd.exe` of every installed Apache. Defender then
//!   skips the files those processes open (every project's PHP sources, wherever they are)
//!   without excluding any source folder. `mysqld.exe` is not named: the data folder already
//!   covers what it writes, and `mysql` must stay what it always was.
//!
//! Only on an explicit request: nothing adds an exclusion at install or start. Each request shows
//! one UAC prompt whatever it names, so a call that may not prompt (the CLI without
//! `--allow-elevation`) is refused before anything happens. The executables are those installed
//! when the request is made: a PHP installed later needs a new request.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lampo_domain::package::{EntryRole, PackageKind};
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{InstalledPackageRegistry, ScanExclusionEditor};
use lampo_domain::scan_exclusion::{ScanExclusionChange, ScanExclusionOutcome, ScanExclusionTargets};
use lampo_domain::service::ServiceKind;

use crate::context::{CallContext, ElevationPolicy};
use crate::dto::{DefenderExclusionInfoDto, DefenderExclusionRequestDto, DefenderExclusionResultDto, ScanExclusionChangeDto, ScanExclusionOutcomeDto, ScanExclusionResultDto};
use crate::error::{AppError, AppResult, ErrorCode};

/// The server executable of each package kind the `php` part names.
const SERVER_EXECUTABLES: [(PackageKind, EntryRole); 2] = [(PackageKind::PHP, EntryRole::FastCgi), (PackageKind::APACHE, EntryRole::Server)];

pub struct DefenderExclusion {
    editor: Arc<dyn ScanExclusionEditor>,
    installed: Arc<dyn InstalledPackageRegistry>,
    paths: LampoPaths,
    /// One prompt at a time.
    busy: tokio::sync::Mutex<()>,
}

impl DefenderExclusion {
    pub fn new(editor: Arc<dyn ScanExclusionEditor>, installed: Arc<dyn InstalledPackageRegistry>, paths: LampoPaths) -> Self {
        Self { editor, installed, paths, busy: tokio::sync::Mutex::new(()) }
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

    /// What `mysql` and `php` would cover now.
    pub fn info(&self) -> AppResult<DefenderExclusionInfoDto> {
        Ok(DefenderExclusionInfoDto { folder: display(&self.folder()), processes: self.server_executables()?.iter().map(|p| display(p)).collect() })
    }

    /// Asks Defender to add or remove the exclusions `request` names, in one UAC prompt.
    /// `Validation` when it names nothing, `ElevationRequired` when `context` may not prompt,
    /// `PackageMissing` for `php` without any PHP or Apache installed; the outcomes the user can
    /// act on are results, not errors.
    pub async fn change(&self, request: DefenderExclusionRequestDto, context: CallContext) -> AppResult<DefenderExclusionResultDto> {
        if !request.php && !request.mysql {
            return Err(AppError::new(ErrorCode::Validation, "name what to exclude: --php, --mysql or both"));
        }
        if context.elevation == ElevationPolicy::Deny {
            return Err(AppError::new(ErrorCode::ElevationRequired, "changing the Microsoft Defender exclusions shows a UAC prompt: re-run with --allow-elevation"));
        }
        let _busy = self.busy.lock().await;
        let processes = if request.php { self.server_executables()? } else { Vec::new() };
        if request.php && processes.is_empty() {
            return Err(AppError::new(ErrorCode::PackageMissing, "no PHP or Apache package is installed: there is no php-cgi.exe or httpd.exe to exclude"));
        }
        let targets = ScanExclusionTargets { folder: request.mysql.then(|| self.folder()), processes };
        let outcome = self.editor.change(&targets, change_from_dto(request.change)).await?;
        Ok(DefenderExclusionResultDto {
            change: request.change,
            outcome: outcome_to_dto(outcome),
            folder: targets.folder.as_deref().map(display),
            processes: targets.processes.iter().map(|p| display(p)).collect(),
        })
    }

    /// The MySQL-only form of [`Self::change`].
    pub async fn change_mysql(&self, change: ScanExclusionChangeDto, context: CallContext) -> AppResult<ScanExclusionResultDto> {
        let result = self.change(DefenderExclusionRequestDto { change, php: false, mysql: true }, context).await?;
        Ok(ScanExclusionResultDto { change, outcome: result.outcome, folder: display(&self.folder()) })
    }
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
