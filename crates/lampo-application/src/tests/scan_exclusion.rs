//! The optional Defender exclusion of the MySQL data folder.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::ScanExclusionEditor;
use lampo_domain::scan_exclusion::{ScanExclusionChange, ScanExclusionOutcome};
use lampo_domain::{PortError, PortResult};

use super::{cli, gui};
use crate::context::CallContext;
use crate::dto::*;
use crate::error::ErrorCode;
use crate::use_cases::MysqlScanExclusion;

/// Records each request and answers with a fixed result.
struct FakeEditor {
    result: PortResult<ScanExclusionOutcome>,
    calls: Mutex<Vec<(PathBuf, ScanExclusionChange)>>,
}

#[async_trait]
impl ScanExclusionEditor for FakeEditor {
    async fn change(&self, folder: &Path, change: ScanExclusionChange) -> PortResult<ScanExclusionOutcome> {
        self.calls.lock().unwrap().push((folder.to_path_buf(), change));
        self.result.clone()
    }
}

fn setup(result: PortResult<ScanExclusionOutcome>) -> (Arc<FakeEditor>, MysqlScanExclusion) {
    let editor = Arc::new(FakeEditor { result, calls: Mutex::new(Vec::new()) });
    let use_case = MysqlScanExclusion::new(editor.clone(), LampoPaths::new(r"C:\Users\me\AppData\Roaming\Lampo", r"C:\Users\me\AppData\Local\Lampo"));
    (editor, use_case)
}

const MYSQL_FOLDER: &str = r"C:\Users\me\AppData\Local\Lampo\mysql";

#[tokio::test]
async fn only_the_mysql_data_folder_is_named() {
    let (editor, use_case) = setup(Ok(ScanExclusionOutcome::Applied));
    assert_eq!(use_case.info().folder, MYSQL_FOLDER);

    let added = use_case.change(ScanExclusionChangeDto::Add, gui()).await.unwrap();
    assert_eq!(added, ScanExclusionResultDto { change: ScanExclusionChangeDto::Add, outcome: ScanExclusionOutcomeDto::Applied, folder: MYSQL_FOLDER.into() });
    let removed = use_case.change(ScanExclusionChangeDto::Remove, CallContext::cli(true)).await.unwrap();
    assert_eq!(removed.change, ScanExclusionChangeDto::Remove);

    let calls = editor.calls.lock().unwrap();
    assert_eq!(*calls, vec![(PathBuf::from(MYSQL_FOLDER), ScanExclusionChange::Add), (PathBuf::from(MYSQL_FOLDER), ScanExclusionChange::Remove)]);
}

#[tokio::test]
async fn the_cli_never_prompts_without_allow_elevation() {
    let (editor, use_case) = setup(Ok(ScanExclusionOutcome::Applied));
    for change in [ScanExclusionChangeDto::Add, ScanExclusionChangeDto::Remove] {
        let err = use_case.change(change, cli()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::ElevationRequired);
        assert!(err.message.contains("--allow-elevation"), "{}", err.message);
    }
    assert!(editor.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn outcomes_the_user_can_act_on_are_results() {
    for (outcome, dto) in [
        (ScanExclusionOutcome::Cancelled, ScanExclusionOutcomeDto::Cancelled),
        (ScanExclusionOutcome::DefenderUnavailable, ScanExclusionOutcomeDto::DefenderUnavailable),
        (ScanExclusionOutcome::Refused, ScanExclusionOutcomeDto::Refused),
        (ScanExclusionOutcome::NotConfirmed, ScanExclusionOutcomeDto::NotConfirmed),
    ] {
        let (_editor, use_case) = setup(Ok(outcome));
        assert_eq!(use_case.change(ScanExclusionChangeDto::Add, gui()).await.unwrap().outcome, dto, "{outcome:?}");
    }
}

#[tokio::test]
async fn adapter_failures_are_errors() {
    let (_editor, use_case) = setup(Err(PortError::invalid_data(MYSQL_FOLDER, "only the MySQL data folder of a Windows user profile can be excluded")));
    assert_eq!(use_case.change(ScanExclusionChangeDto::Add, gui()).await.unwrap_err().code, ErrorCode::Validation);
    let (_editor, use_case) = setup(Err(PortError::not_found("elevated helper", "lampo-hosts-helper.exe")));
    assert_eq!(use_case.change(ScanExclusionChangeDto::Remove, gui()).await.unwrap_err().code, ErrorCode::NotFound);
}

#[test]
fn the_wire_format_is_kebab_case() {
    let result = ScanExclusionResultDto { change: ScanExclusionChangeDto::Remove, outcome: ScanExclusionOutcomeDto::DefenderUnavailable, folder: MYSQL_FOLDER.into() };
    assert_eq!(serde_json::to_value(&result).unwrap(), serde_json::json!({ "change": "remove", "outcome": "defender-unavailable", "folder": MYSQL_FOLDER }));
}
