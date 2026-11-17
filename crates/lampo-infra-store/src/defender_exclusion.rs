//! `%LOCALAPPDATA%\Lampo\defender-exclusion.toml`: what Lampo remembers of its Defender
//! exclusion requests for the server processes (`LampoPaths::defender_exclusion_file`).
//!
//! ```toml
//! excluded = ['C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe']
//!
//! [last_request]
//! change = "add"
//! outcome = "applied"
//! processes = ['C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe']
//! ```

use std::path::PathBuf;

use lampo_domain::PortResult;
use lampo_domain::ports::{ProcessExclusionRecordReader, ProcessExclusionRecordWriter};
use lampo_domain::scan_exclusion::{ProcessExclusionRecord, ProcessExclusionRequest, ScanExclusionChange, ScanExclusionOutcome};
use serde::{Deserialize, Serialize};

use crate::atomic::{read_toml, write_toml};

/// The record in its file; missing file → never asked.
pub struct TomlProcessExclusionRecord {
    file: PathBuf,
}

impl TomlProcessExclusionRecord {
    /// `file` = `LampoPaths::defender_exclusion_file()`.
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }
}

impl ProcessExclusionRecordReader for TomlProcessExclusionRecord {
    fn read(&self) -> PortResult<ProcessExclusionRecord> {
        Ok(read_toml::<RecordFile>(&self.file)?.map(ProcessExclusionRecord::from).unwrap_or_default())
    }
}

impl ProcessExclusionRecordWriter for TomlProcessExclusionRecord {
    fn write(&self, record: &ProcessExclusionRecord) -> PortResult<()> {
        write_toml(&self.file, &RecordFile::from(record))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct RecordFile {
    #[serde(default)]
    excluded: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_request: Option<RequestFile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RequestFile {
    change: ChangeFile,
    outcome: OutcomeFile,
    #[serde(default)]
    processes: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ChangeFile {
    Add,
    Remove,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum OutcomeFile {
    Applied,
    Cancelled,
    DefenderUnavailable,
    Refused,
    NotConfirmed,
}

impl From<RecordFile> for ProcessExclusionRecord {
    fn from(file: RecordFile) -> Self {
        Self { last_request: file.last_request.map(ProcessExclusionRequest::from), excluded: file.excluded }
    }
}

impl From<&ProcessExclusionRecord> for RecordFile {
    fn from(record: &ProcessExclusionRecord) -> Self {
        Self { excluded: record.excluded.clone(), last_request: record.last_request.as_ref().map(RequestFile::from) }
    }
}

impl From<RequestFile> for ProcessExclusionRequest {
    fn from(file: RequestFile) -> Self {
        Self { change: file.change.into(), outcome: file.outcome.into(), processes: file.processes }
    }
}

impl From<&ProcessExclusionRequest> for RequestFile {
    fn from(request: &ProcessExclusionRequest) -> Self {
        Self { change: request.change.into(), outcome: request.outcome.into(), processes: request.processes.clone() }
    }
}

impl From<ChangeFile> for ScanExclusionChange {
    fn from(change: ChangeFile) -> Self {
        match change {
            ChangeFile::Add => Self::Add,
            ChangeFile::Remove => Self::Remove,
        }
    }
}

impl From<ScanExclusionChange> for ChangeFile {
    fn from(change: ScanExclusionChange) -> Self {
        match change {
            ScanExclusionChange::Add => Self::Add,
            ScanExclusionChange::Remove => Self::Remove,
        }
    }
}

impl From<OutcomeFile> for ScanExclusionOutcome {
    fn from(outcome: OutcomeFile) -> Self {
        match outcome {
            OutcomeFile::Applied => Self::Applied,
            OutcomeFile::Cancelled => Self::Cancelled,
            OutcomeFile::DefenderUnavailable => Self::DefenderUnavailable,
            OutcomeFile::Refused => Self::Refused,
            OutcomeFile::NotConfirmed => Self::NotConfirmed,
        }
    }
}

impl From<ScanExclusionOutcome> for OutcomeFile {
    fn from(outcome: ScanExclusionOutcome) -> Self {
        match outcome {
            ScanExclusionOutcome::Applied => Self::Applied,
            ScanExclusionOutcome::Cancelled => Self::Cancelled,
            ScanExclusionOutcome::DefenderUnavailable => Self::DefenderUnavailable,
            ScanExclusionOutcome::Refused => Self::Refused,
            ScanExclusionOutcome::NotConfirmed => Self::NotConfirmed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::paths;

    fn store(dir: &tempfile::TempDir) -> TomlProcessExclusionRecord {
        TomlProcessExclusionRecord::new(paths(dir).defender_exclusion_file())
    }

    #[test]
    fn a_missing_file_is_a_request_never_made() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(store(&dir).read().unwrap(), ProcessExclusionRecord::default());
    }

    #[test]
    fn every_outcome_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let php = PathBuf::from(r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe");
        let outcomes =
            [ScanExclusionOutcome::Applied, ScanExclusionOutcome::Cancelled, ScanExclusionOutcome::DefenderUnavailable, ScanExclusionOutcome::Refused, ScanExclusionOutcome::NotConfirmed];
        for (change, outcome) in [ScanExclusionChange::Add, ScanExclusionChange::Remove].into_iter().flat_map(|change| outcomes.map(|outcome| (change, outcome))) {
            let record = ProcessExclusionRecord { last_request: Some(ProcessExclusionRequest { change, outcome, processes: vec![php.clone()] }), excluded: vec![php.clone()] };
            store.write(&record).unwrap();
            assert_eq!(store.read().unwrap(), record, "{change:?} {outcome:?}");
        }
        store.write(&ProcessExclusionRecord::default()).unwrap();
        assert_eq!(store.read().unwrap(), ProcessExclusionRecord::default());
    }

    #[test]
    fn a_corrupt_file_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = paths(&dir).defender_exclusion_file();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "[last_request]\nchange = \"sideways\"\n").unwrap();
        assert!(store(&dir).read().unwrap_err().to_string().contains("defender-exclusion.toml"));
    }
}
