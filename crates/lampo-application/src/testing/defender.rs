//! Fakes of the Defender ports the doctor and the exclusion requests use.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lampo_domain::ports::{DefenderStateReader, ProcessExclusionRecordReader, ProcessExclusionRecordWriter, ScanExclusionEditor};
use lampo_domain::scan_exclusion::{DefenderState, ProcessExclusionRecord, ScanExclusionChange, ScanExclusionOutcome, ScanExclusionTargets};
use lampo_domain::{PortError, PortResult};

use super::{World, lock};
use crate::use_cases::DefenderExclusion;

/// Answers with a fixed state; by default Defender is not the active antivirus (no finding).
pub struct FakeDefender(Mutex<PortResult<DefenderState>>);

impl Default for FakeDefender {
    fn default() -> Self {
        Self(Mutex::new(Ok(DefenderState::Unavailable)))
    }
}

impl FakeDefender {
    pub fn set(&self, state: DefenderState) {
        *lock(&self.0) = Ok(state);
    }

    /// The state cannot be read (PowerShell failing or timing out).
    pub fn unreadable(&self) {
        *lock(&self.0) = Err(PortError::process("powershell", "did not answer in time"));
    }
}

#[async_trait]
impl DefenderStateReader for FakeDefender {
    async fn state(&self) -> PortResult<DefenderState> {
        lock(&self.0).clone()
    }
}

/// The record in memory; [`Self::corrupt`] makes reads fail as an unloadable file does.
#[derive(Default)]
pub struct MemExclusionRecord {
    record: Mutex<ProcessExclusionRecord>,
    corrupt: Mutex<bool>,
}

impl MemExclusionRecord {
    pub fn get(&self) -> ProcessExclusionRecord {
        lock(&self.record).clone()
    }

    pub fn corrupt(&self) {
        *lock(&self.corrupt) = true;
    }
}

impl ProcessExclusionRecordReader for MemExclusionRecord {
    fn read(&self) -> PortResult<ProcessExclusionRecord> {
        match *lock(&self.corrupt) {
            true => Err(PortError::invalid_data("defender-exclusion.toml", "corrupt")),
            false => Ok(self.get()),
        }
    }
}

impl ProcessExclusionRecordWriter for MemExclusionRecord {
    fn write(&self, record: &ProcessExclusionRecord) -> PortResult<()> {
        *lock(&self.record) = record.clone();
        *lock(&self.corrupt) = false;
        Ok(())
    }
}

/// Applies nothing and answers every request with a fixed outcome; accepts every target as given.
pub struct FixedOutcomeEditor(pub ScanExclusionOutcome);

#[async_trait]
impl ScanExclusionEditor for FixedOutcomeEditor {
    async fn change(&self, _targets: &ScanExclusionTargets, _change: ScanExclusionChange) -> PortResult<ScanExclusionOutcome> {
        Ok(self.0)
    }

    fn accepted_projects_folder(&self, folder: &Path) -> PortResult<PathBuf> {
        Ok(folder.to_path_buf())
    }

    fn accepted_targets(&self, targets: &ScanExclusionTargets, _change: ScanExclusionChange) -> PortResult<ScanExclusionTargets> {
        Ok(targets.clone())
    }
}

impl World {
    /// The Defender exclusion requests of this world, answered with `outcome` and remembered in
    /// [`World::exclusion_record`].
    pub fn defender_exclusion(&self, outcome: ScanExclusionOutcome) -> DefenderExclusion {
        DefenderExclusion::new(
            Arc::new(FixedOutcomeEditor(outcome)),
            self.installed.clone(),
            self.projects.clone(),
            self.paths.clone(),
            self.exclusion_record.clone(),
            self.exclusion_record.clone(),
        )
    }
}
