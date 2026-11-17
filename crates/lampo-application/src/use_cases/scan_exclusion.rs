//! The optional Microsoft Defender exclusion of the MySQL data folder (Settings > Strumenti,
//! `lampo mysql defender-exclusion add|remove`).
//!
//! Only the folder holding every MySQL instance's data (`<data root>\mysql`) is ever named, and
//! only on an explicit request: nothing adds the exclusion at install or start. Each request
//! shows a UAC prompt, so a call that may not prompt (the CLI without `--allow-elevation`) is
//! refused before anything happens.

use std::path::PathBuf;
use std::sync::Arc;

use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::ScanExclusionEditor;
use lampo_domain::scan_exclusion::{ScanExclusionChange, ScanExclusionOutcome};
use lampo_domain::service::ServiceKind;

use crate::context::{CallContext, ElevationPolicy};
use crate::dto::{ScanExclusionChangeDto, ScanExclusionInfoDto, ScanExclusionOutcomeDto, ScanExclusionResultDto};
use crate::error::{AppError, AppResult, ErrorCode};

pub struct MysqlScanExclusion {
    editor: Arc<dyn ScanExclusionEditor>,
    paths: LampoPaths,
    /// One prompt at a time.
    busy: tokio::sync::Mutex<()>,
}

impl MysqlScanExclusion {
    pub fn new(editor: Arc<dyn ScanExclusionEditor>, paths: LampoPaths) -> Self {
        Self { editor, paths, busy: tokio::sync::Mutex::new(()) }
    }

    fn folder(&self) -> PathBuf {
        self.paths.service_kind_dir(&ServiceKind::MYSQL)
    }

    pub fn info(&self) -> ScanExclusionInfoDto {
        ScanExclusionInfoDto { folder: self.folder().display().to_string() }
    }

    /// Asks Defender to add or remove the exclusion (UAC prompt). `ElevationRequired` when
    /// `context` may not prompt; the outcomes the user can act on are results, not errors.
    pub async fn change(&self, change: ScanExclusionChangeDto, context: CallContext) -> AppResult<ScanExclusionResultDto> {
        if context.elevation == ElevationPolicy::Deny {
            return Err(AppError::new(ErrorCode::ElevationRequired, "changing the Microsoft Defender exclusion shows a UAC prompt: re-run with --allow-elevation"));
        }
        let _busy = self.busy.lock().await;
        let folder = self.folder();
        let outcome = self.editor.change(&folder, change_from_dto(change)).await?;
        Ok(ScanExclusionResultDto { change, outcome: outcome_to_dto(outcome), folder: folder.display().to_string() })
    }
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
