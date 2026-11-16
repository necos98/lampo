//! Whether the Lampo block of the hosts file lists every project hostname. The one comparison
//! shared by the doctor's `hosts-block` check and the cheap query the GUI polls after every
//! project change (the full doctor probes ports, PATH and runtimes it does not need).

use std::sync::Arc;

use lampo_domain::hosts::HostsBlock;
use lampo_domain::PortResult;
use lampo_domain::ports::{HostsFileReader, ProjectRepository};
use lampo_domain::project::Project;

use crate::error::AppResult;

/// The hosts block mirrors the hostnames of `projects` (a missing block counts as empty).
pub fn hosts_block_in_sync(projects: &[Project], hosts: &dyn HostsFileReader) -> PortResult<bool> {
    let current = hosts.read_block()?.unwrap_or_default();
    Ok(current == HostsBlock::for_projects(projects))
}

/// Read-only: loads the projects and reads the hosts file, nothing else.
pub struct HostsSyncQuery {
    projects: Arc<dyn ProjectRepository>,
    hosts: Arc<dyn HostsFileReader>,
}

impl HostsSyncQuery {
    pub fn new(projects: Arc<dyn ProjectRepository>, hosts: Arc<dyn HostsFileReader>) -> Self {
        Self { projects, hosts }
    }

    /// Fails when the hosts file cannot be read: "unknown" is not "out of sync".
    pub fn in_sync(&self) -> AppResult<bool> {
        let projects = self.projects.list()?;
        Ok(hosts_block_in_sync(&projects, self.hosts.as_ref())?)
    }
}
