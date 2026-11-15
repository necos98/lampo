//! Export of a project's configuration to a portable file, and its import (possibly on another
//! machine, possibly from someone else: the file is untrusted).

use std::path::Path;
use std::sync::Arc;

use lampo_domain::address::SiteAddress;
use lampo_domain::mysql::DatabaseName;
use lampo_domain::ports::{FileSystemInspector, ProjectRepository, ProjectTransfer};
use lampo_domain::project::{TlsMode, unique_project_id};
use lampo_domain::project_import::ImportedProject;

use super::projects::{ProjectWriter, existing_folder, require_hosts_elevation};
use super::support::{load_project, validation};
use crate::context::CallContext;
use crate::dto::{ExportProjectRequest, ImportProjectRequest, ImportProjectResultDto};
use crate::error::AppResult;

pub struct ExportProject {
    projects: Arc<dyn ProjectRepository>,
    transfer: Arc<dyn ProjectTransfer>,
}

impl ExportProject {
    pub fn new(projects: Arc<dyn ProjectRepository>, transfer: Arc<dyn ProjectTransfer>) -> Self {
        Self { projects, transfer }
    }

    pub fn execute(&self, request: ExportProjectRequest) -> AppResult<()> {
        let project = load_project(self.projects.as_ref(), &request.project_id)?;
        if request.file.trim().is_empty() {
            return Err(validation("an export file is required"));
        }
        Ok(self.transfer.export_to(&project, Path::new(request.file.trim()))?)
    }
}

/// Imports an exported config (inactive), optionally re-rooted, de-duplicating the id.
pub struct ImportProject {
    writer: ProjectWriter,
    transfer: Arc<dyn ProjectTransfer>,
    fs: Arc<dyn FileSystemInspector>,
}

impl ImportProject {
    pub fn new(writer: ProjectWriter, transfer: Arc<dyn ProjectTransfer>, fs: Arc<dyn FileSystemInspector>) -> Self {
        Self { writer, transfer, fs }
    }

    /// The imported project is always inactive and without HTTPS. Its custom Apache directives
    /// and the php.ini keys that could run code here are never imported: they are returned in
    /// the result for the user to review. `root` (if given) must exist; without it the exported
    /// folder is kept even if missing here (relink later). When the id is already taken it gets
    /// a numeric suffix and its addresses are regenerated for the new id (the old ones would
    /// collide with the existing project). Exported addresses are otherwise kept and validated
    /// like any other save (`Conflict` when a loopback port belongs to another site). A project
    /// with hosts-file names needs elevation (`ElevationRequired`, nothing imported, otherwise).
    pub async fn execute(&self, request: ImportProjectRequest, ctx: &CallContext) -> AppResult<ImportProjectResultDto> {
        if request.file.trim().is_empty() {
            return Err(validation("an export file is required"));
        }
        let ImportedProject { mut project, dropped } = self.transfer.import_from(Path::new(request.file.trim()))?;
        if let Some(raw) = request.root.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            project.relink(existing_folder(self.fs.as_ref(), raw)?);
        }
        project.set_active(false);
        project.tls = TlsMode::Disabled;
        let project = self
            .writer
            .allocator
            .reserve(|snap| {
                let id = unique_project_id(&snap.projects, &project.id);
                if id != project.id {
                    project.set_addresses(SiteAddress::defaults_for(id.as_str(), &snap.settings.tld)?)?;
                    if project.database.is_some() {
                        project.database = Some(DatabaseName::from_slug(id.as_str())?);
                    }
                    project.id = id;
                }
                let names: Vec<String> = project.hosts_file_names().map(ToString::to_string).collect();
                require_hosts_elevation(ctx, &names, &format!("importing `{}`", project.id), "nothing was imported")?;
                self.writer.save_checked(snap, &project, true)?;
                Ok(project)
            })
            .await?;
        self.writer.changed(&project.id);
        let project = self.writer.finish(&project, ctx).await?;
        Ok(ImportProjectResultDto { project, dropped_apache_directives: dropped.apache_directives, dropped_php_ini_keys: dropped.php_ini_keys })
    }
}
