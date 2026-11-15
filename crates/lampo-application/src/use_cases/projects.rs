//! Project CRUD and lifecycle. Config only: these never touch the project folder or `.env`.
//!
//! Every use case that saves a project does it through [`ProjectWriter`]: under the shared
//! [`LoopbackPortAllocator`] lock, after `project::check_conflicts` against every project *and
//! every worktree environment*. The rule cannot be skipped by forgetting an optional builder.

use std::path::Path;
use std::sync::Arc;

use lampo_domain::address::SiteAddress;
use lampo_domain::detection::combine_hints;
use lampo_domain::events::DomainEvent;
use lampo_domain::mysql::DatabaseName;
use lampo_domain::package::PackageKind;
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::{EventPublisher, FileSystemInspector, InstalledPackageRegistry, ProjectDetector, ProjectRepository, ProjectTransfer, SettingsRepository, WorktreeRepository};
use lampo_domain::preset::Preset;
use lampo_domain::project::{DisplayName, Project, ProjectEnvironment, ProjectId, ProjectRoot, TlsMode, check_conflicts, unique_project_id};
use lampo_domain::service::ServiceKind;

use super::allocation::{LoopbackPortAllocator, SiteSnapshot};
use super::database::DatabaseService;
use super::environment::EnvironmentSync;
use super::presets::PresetQueries;
use super::service_catalog::ServiceCatalog;
use super::support::{load_project, newest_of, package_missing, parse_project_id, project_dto, validation};
use crate::context::{CallContext, ElevationPolicy};
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mapping::{address_from_dto, address_to_dto, environment_from_dto};

/// Preset used when neither the request nor detection names one.
pub const FALLBACK_PRESET: &str = "generic-php";

fn folder_name(root: &ProjectRoot) -> String {
    root.as_str().trim_end_matches('\\').rsplit('\\').next().filter(|s| !s.is_empty() && !s.ends_with(':')).unwrap_or("project").to_string()
}

/// An existing absolute folder, or `Validation`.
fn existing_folder(fs: &dyn FileSystemInspector, raw: &str) -> AppResult<ProjectRoot> {
    let root = ProjectRoot::new(raw)?;
    if !fs.is_dir(root.as_path()) {
        return Err(validation(format!("folder `{root}` does not exist")));
    }
    Ok(root)
}

/// Read-only project views (works without the app).
pub struct ProjectQueries {
    projects: Arc<dyn ProjectRepository>,
    worktrees: Arc<dyn WorktreeRepository>,
    settings: Arc<dyn SettingsRepository>,
    fs: Arc<dyn FileSystemInspector>,
}

impl ProjectQueries {
    pub fn new(projects: Arc<dyn ProjectRepository>, worktrees: Arc<dyn WorktreeRepository>, settings: Arc<dyn SettingsRepository>, fs: Arc<dyn FileSystemInspector>) -> Self {
        Self { projects, worktrees, settings, fs }
    }

    pub fn list(&self) -> AppResult<Vec<ProjectDto>> {
        let settings = self.settings.load()?;
        let worktrees = self.worktrees.list_all()?;
        Ok(self.projects.list()?.iter().map(|p| project_dto(p, &worktrees, &settings, Some(self.fs.as_ref()))).collect())
    }

    pub fn summaries(&self) -> AppResult<Vec<ProjectSummaryDto>> {
        let settings = self.settings.load()?;
        let worktrees = self.worktrees.list_all()?;
        Ok(self
            .projects
            .list()?
            .iter()
            .map(|p| ProjectSummaryDto {
                id: p.id.to_string(),
                name: p.name.to_string(),
                root: p.root.to_string(),
                active: p.active,
                url: p.url(&settings),
                php_version: p.environment.php.to_string(),
                mysql_version: p.environment.mysql().map(|m| m.to_string()),
                worktree_count: worktrees.iter().filter(|w| w.parent == p.id).count(),
            })
            .collect())
    }

    /// `NotFound` if unknown.
    pub fn get(&self, project_id: &str) -> AppResult<ProjectDto> {
        let project = load_project(self.projects.as_ref(), project_id)?;
        Ok(project_dto(&project, &self.worktrees.list_for(&project.id)?, &self.settings.load()?, Some(self.fs.as_ref())))
    }
}

/// Analyses a folder: runs every detector, combines their hints (`combine_hints`).
pub struct DetectProject {
    detectors: Vec<Arc<dyn ProjectDetector>>,
    projects: Arc<dyn ProjectRepository>,
    fs: Arc<dyn FileSystemInspector>,
}

impl DetectProject {
    pub fn new(detectors: Vec<Arc<dyn ProjectDetector>>, projects: Arc<dyn ProjectRepository>, fs: Arc<dyn FileSystemInspector>) -> Self {
        Self { detectors, projects, fs }
    }

    /// `Validation` if `folder` is not an existing absolute directory. A failing detector is
    /// logged and skipped.
    pub fn execute(&self, folder: &str) -> AppResult<DetectionDto> {
        let root = existing_folder(self.fs.as_ref(), folder)?;
        let hints = self
            .detectors
            .iter()
            .filter_map(|d| match d.detect(root.as_path()) {
                Ok(hint) => hint,
                Err(e) => {
                    tracing::warn!("detector `{}` failed on {root}: {e}", d.name());
                    None
                }
            })
            .collect();
        let best = combine_hints(hints);
        let existing = self.projects.list()?;
        let name = folder_name(&root);
        Ok(DetectionDto {
            folder: root.to_string(),
            project_type: best.as_ref().map(|h| h.project_type.clone()),
            preset_id: best.as_ref().map(|h| h.preset.to_string()),
            php_version: best.as_ref().and_then(|h| h.php).map(|p| p.to_string()),
            suggested_id: unique_project_id(&existing, &ProjectId::from_name(&name)).to_string(),
            suggested_name: name,
            services: best.as_ref().map(|h| h.services.iter().map(ToString::to_string).collect()).unwrap_or_default(),
            warnings: best.as_ref().map(|h| h.warnings.clone()).unwrap_or_default(),
            evidence: best.map(|h| h.evidence).unwrap_or_default(),
            already_registered: existing.iter().find(|p| p.root == root).map(|p| p.id.to_string()),
        })
    }
}

/// What every project-mutating use case needs: the repository, the allocator owning the
/// loopback-port lock, the environment sync, events and the full project view. All required.
#[derive(Clone)]
pub struct ProjectWriter {
    pub projects: Arc<dyn ProjectRepository>,
    pub allocator: Arc<LoopbackPortAllocator>,
    pub env: Arc<dyn EnvironmentSync>,
    pub events: Arc<dyn EventPublisher>,
    pub queries: Arc<ProjectQueries>,
    /// Registered service kinds: a project may only use these.
    pub services: Arc<ServiceCatalog>,
}

impl ProjectWriter {
    /// Validates `candidate` against every project and worktree environment (`check_conflicts`)
    /// and saves it. Call only inside an allocator commit (`snap` comes from it).
    fn save_checked(&self, snap: &SiteSnapshot, candidate: &Project, is_new: bool) -> AppResult<()> {
        self.services.validate_environment(&candidate.environment)?;
        check_conflicts(&snap.projects, &snap.worktrees, candidate, is_new, &snap.settings)?;
        self.projects.save(candidate)?;
        Ok(())
    }

    /// Loads `project_id`, applies `change` and, if it reports a change, saves the result, all
    /// under the allocator lock (no lost update, no conflict slipping in).
    pub(crate) async fn modify(&self, project_id: &str, change: impl FnOnce(&mut Project, &SiteSnapshot) -> AppResult<bool>) -> AppResult<Project> {
        self.allocator.reserve(|snap| self.commit_change(project_id, snap, change)).await
    }

    /// Like [`Self::modify`], also handing `change` a free loopback port picked under the same
    /// lock (it may leave it unused).
    pub(crate) async fn modify_with_free_port(&self, project_id: &str, change: impl FnOnce(&mut Project, &SiteSnapshot, u16) -> AppResult<bool>) -> AppResult<Project> {
        self.allocator.allocate(|port, snap| self.commit_change(project_id, snap, |project, snap| change(project, snap, port))).await
    }

    fn commit_change(&self, project_id: &str, snap: &SiteSnapshot, change: impl FnOnce(&mut Project, &SiteSnapshot) -> AppResult<bool>) -> AppResult<Project> {
        let mut project = load_project(self.projects.as_ref(), project_id)?;
        if change(&mut project, snap)? {
            self.save_checked(snap, &project, false)?;
            self.changed(&project.id);
        }
        Ok(project)
    }

    fn changed(&self, id: &ProjectId) {
        self.events.publish(DomainEvent::ProjectsChanged { changed: vec![id.clone()] });
    }

    /// Syncs, then returns the full view of `project`.
    pub(crate) async fn finish(&self, project: &Project, ctx: &CallContext) -> AppResult<ProjectDto> {
        self.env.sync(ctx).await?;
        self.queries.get(project.id.as_str())
    }
}

/// Registers a folder as a project (copying a preset), creates its MySQL database, then syncs
/// the environment.
pub struct CreateProject {
    writer: ProjectWriter,
    presets: Arc<PresetQueries>,
    detect: Arc<DetectProject>,
    fs: Arc<dyn FileSystemInspector>,
    databases: Arc<DatabaseService>,
}

impl CreateProject {
    pub fn new(writer: ProjectWriter, presets: Arc<PresetQueries>, detect: Arc<DetectProject>, fs: Arc<dyn FileSystemInspector>, databases: Arc<DatabaseService>) -> Self {
        Self { writer, presets, detect, fs, databases }
    }

    /// The preset to copy: requested, else detected (if it exists), else [`FALLBACK_PRESET`].
    fn choose_preset(&self, requested: Option<&str>, detected: Option<&str>) -> AppResult<Preset> {
        if let Some(id) = requested {
            return self.presets.find(id);
        }
        if let Some(found) = detected.and_then(|id| self.presets.find(id).ok()) {
            return Ok(found);
        }
        self.presets.find(FALLBACK_PRESET)
    }

    /// Adds the detected services (a Docker Compose `redis`…) the environment lacks: only
    /// registered, unversioned kinds (a versioned one, MySQL, keeps the preset's series).
    fn add_detected_services(&self, environment: &mut ProjectEnvironment, detected: &[String]) {
        for kind in detected.iter().filter_map(|k| ServiceKind::new(k.as_str()).ok()) {
            let known = self.writer.services.descriptor(&kind).is_some_and(|d| !d.versioned());
            if known && !environment.uses(&kind) {
                environment.use_service(kind, None);
            }
        }
    }

    /// Fills omitted fields (see [`CreateProjectRequest`]), allocates a loopback port for
    /// `AddressingDto::NoElevation` (atomically with the save), checks conflicts, saves,
    /// publishes `ProjectsChanged`, creates the database (when the project uses MySQL and
    /// `create_database`), syncs.
    ///
    /// PHP: the request's `phpVersion`, else the preset's — raised to the detected minimum
    /// (`composer.json` `require.php`, a container image) when the preset's PHP is older than that.
    /// Services: the preset's, plus the detected unversioned ones (`add_detected_services`).
    ///
    /// The database step never fails the creation (the project is already saved): its outcome
    /// is in [`CreateProjectResultDto::database_setup`]. It runs before the sync, so the sync
    /// leaves the MySQL instance in the state the active projects need.
    pub async fn execute(&self, request: CreateProjectRequest, ctx: &CallContext) -> AppResult<CreateProjectResultDto> {
        let root = existing_folder(self.fs.as_ref(), &request.root)?;
        let detection = self.detect.execute(root.as_str())?;
        let name = DisplayName::new(request.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&detection.suggested_name))?;
        let requested_id = request.id.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(parse_project_id).transpose()?;

        let preset = self.choose_preset(request.preset_id.as_deref(), detection.preset_id.as_deref())?;
        let mut environment = match &request.environment {
            Some(dto) => environment_from_dto(dto)?,
            None => preset.environment.clone(),
        };
        match request.php_version.as_deref() {
            Some(php) => environment.php = PhpMinor::parse(php)?,
            None if request.environment.is_none() => {
                if let Some(detected) = detection.php_version.as_deref().and_then(|p| PhpMinor::parse(p).ok()) {
                    environment.php = environment.php.max(detected);
                }
            }
            None => {}
        }
        if request.environment.is_none() {
            self.add_detected_services(&mut environment, &detection.services);
        }
        let explicit = request.addresses.as_ref().map(|list| list.iter().map(address_from_dto).collect::<Result<Vec<_>, _>>()).transpose()?;

        let build = |snap: &SiteSnapshot, port: Option<u16>| -> AppResult<Project> {
            let id = requested_id.clone().unwrap_or_else(|| unique_project_id(&snap.projects, &ProjectId::from_name(name.as_str())));
            let addresses = match (&explicit, port) {
                (Some(list), _) => list.clone(),
                (None, Some(port)) => vec![SiteAddress::localhost(id.as_str())?, SiteAddress::loopback_port(port)?],
                (None, None) => SiteAddress::defaults_for(id.as_str(), &snap.settings.tld)?,
            };
            let mut project = Project::new(id, name.clone(), root.clone(), addresses, environment.clone())?;
            project.origin_preset = Some(preset.id.clone());
            project.set_active(request.activate);
            self.writer.save_checked(snap, &project, true)?;
            Ok(project)
        };
        let project = match (&explicit, request.addressing) {
            (None, AddressingDto::NoElevation) => self.writer.allocator.allocate(|port, snap| build(snap, Some(port))).await?,
            _ => self.writer.allocator.reserve(|snap| build(snap, None)).await?,
        };
        self.writer.changed(&project.id);
        let database_setup = if request.create_database { self.databases.provision_project(&project).await } else { None };
        let project = self.writer.finish(&project, ctx).await?;
        Ok(CreateProjectResultDto { project, database_setup })
    }
}

pub struct UpdateProject {
    writer: ProjectWriter,
}

impl UpdateProject {
    pub fn new(writer: ProjectWriter) -> Self {
        Self { writer }
    }

    /// `Unsupported` for `tls: local-ca` until the local CA exists.
    pub async fn execute(&self, request: UpdateProjectRequest, ctx: &CallContext) -> AppResult<ProjectDto> {
        if request.tls == TlsModeDto::LocalCa {
            return Err(AppError::new(ErrorCode::Unsupported, "HTTPS with a local certificate authority is not available yet"));
        }
        let name = DisplayName::new(&request.name)?;
        let addresses = request.addresses.iter().map(address_from_dto).collect::<Result<Vec<_>, _>>()?;
        let environment = environment_from_dto(&request.environment)?;
        let database = request.database_name.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(DatabaseName::new).transpose()?;
        let project = self
            .writer
            .modify(&request.project_id, |project, _| {
                project.name = name;
                project.set_addresses(addresses)?;
                project.set_environment(environment)?;
                if project.environment.mysql().is_some() {
                    if let Some(db) = database {
                        project.database = Some(db);
                    }
                }
                project.tls = TlsMode::Disabled;
                Ok(true)
            })
            .await?;
        self.writer.finish(&project, ctx).await
    }
}

/// A CLI change of hosts-file `names` (added or removed) is refused up front without
/// `--allow-elevation`, before anything is saved: the sync would apply the rest and fail anyway.
pub(crate) fn require_hosts_elevation(ctx: &CallContext, names: &[String], action: &str, outcome: &str) -> AppResult<()> {
    if ctx.elevation == ElevationPolicy::Allow || names.is_empty() {
        return Ok(());
    }
    let list = names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
    Err(AppError::new(ErrorCode::ElevationRequired, format!("{action} changes the hosts file ({list}), which needs administrator permission: re-run with --allow-elevation ({outcome})")))
}

/// Removes the project's config and its worktree environments' configs. Never deletes folders
/// or databases.
pub struct DeleteProject {
    projects: Arc<dyn ProjectRepository>,
    worktrees: Arc<dyn WorktreeRepository>,
    env: Arc<dyn EnvironmentSync>,
    events: Arc<dyn EventPublisher>,
}

impl DeleteProject {
    pub fn new(projects: Arc<dyn ProjectRepository>, worktrees: Arc<dyn WorktreeRepository>, env: Arc<dyn EnvironmentSync>, events: Arc<dyn EventPublisher>) -> Self {
        Self { projects, worktrees, env, events }
    }

    /// Active projects are removed too (the sync stops serving them). A project with hosts-file
    /// names needs elevation (`ElevationRequired` with nothing removed otherwise), so a retry
    /// with `--allow-elevation` finds it still there.
    pub async fn execute(&self, project_id: &str, ctx: &CallContext) -> AppResult<()> {
        let project = load_project(self.projects.as_ref(), project_id)?;
        let names: Vec<String> = project.hosts_file_names().map(ToString::to_string).collect();
        require_hosts_elevation(ctx, &names, &format!("removing `{}`", project.id), "nothing was removed")?;
        let had_worktrees = !self.worktrees.list_for(&project.id)?.is_empty();
        self.worktrees.delete_all_for(&project.id)?;
        self.projects.delete(&project.id)?;
        self.events.publish(DomainEvent::ProjectsChanged { changed: vec![project.id.clone()] });
        if had_worktrees {
            self.events.publish(DomainEvent::WorktreesChanged { project: project.id.clone() });
        }
        self.env.sync(ctx).await?;
        Ok(())
    }
}

pub struct SetProjectActive {
    writer: ProjectWriter,
}

impl SetProjectActive {
    pub fn new(writer: ProjectWriter) -> Self {
        Self { writer }
    }

    /// Idempotent. Never changes the hosts file (the block lists active and inactive projects).
    pub async fn execute(&self, request: SetProjectActiveRequest, ctx: &CallContext) -> AppResult<ProjectDto> {
        let project = self
            .writer
            .modify(&request.project_id, |project, _| {
                let changed = project.active != request.active;
                project.set_active(request.active);
                Ok(changed)
            })
            .await?;
        self.writer.finish(&project, ctx).await
    }
}

/// Switches a project to another PHP minor (`PackageMissing` if not installed).
pub struct SetProjectPhp {
    writer: ProjectWriter,
    installed: Arc<dyn InstalledPackageRegistry>,
}

impl SetProjectPhp {
    pub fn new(writer: ProjectWriter, installed: Arc<dyn InstalledPackageRegistry>) -> Self {
        Self { writer, installed }
    }

    pub async fn execute(&self, request: SetProjectPhpRequest, ctx: &CallContext) -> AppResult<ProjectDto> {
        let php = PhpMinor::parse(&request.php_version)?;
        if newest_of(&self.installed.list()?, &PackageKind::PHP, Some(php.as_series()), None).is_none() {
            load_project(self.writer.projects.as_ref(), &request.project_id)?;
            return Err(package_missing(format!("PHP {php}")));
        }
        let project = self
            .writer
            .modify(&request.project_id, |project, _| {
                if project.environment.php == php {
                    return Ok(false);
                }
                let mut environment = project.environment.clone();
                environment.php = php;
                project.set_environment(environment)?;
                Ok(true)
            })
            .await?;
        self.writer.finish(&project, ctx).await
    }
}

/// Points a project at a moved folder, keeping its id and config.
pub struct RelinkProject {
    writer: ProjectWriter,
    fs: Arc<dyn FileSystemInspector>,
}

impl RelinkProject {
    pub fn new(writer: ProjectWriter, fs: Arc<dyn FileSystemInspector>) -> Self {
        Self { writer, fs }
    }

    /// `Validation` if the new folder does not exist, `Conflict` if another site uses it or one
    /// of the project's addresses.
    pub async fn execute(&self, request: RelinkProjectRequest, ctx: &CallContext) -> AppResult<ProjectDto> {
        let root = existing_folder(self.fs.as_ref(), &request.root)?;
        let project = self
            .writer
            .modify(&request.project_id, |project, _| {
                project.relink(root);
                Ok(true)
            })
            .await?;
        self.writer.finish(&project, ctx).await
    }
}

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

    /// The imported project is always inactive. `root` (if given) must exist; without it the
    /// exported folder is kept even if missing here (relink later). When the id is already
    /// taken it gets a numeric suffix and its addresses are regenerated for the new id (the old
    /// ones would collide with the existing project). Exported addresses are otherwise kept and
    /// validated like any other save (`Conflict` when a loopback port belongs to another site).
    pub async fn execute(&self, request: ImportProjectRequest, ctx: &CallContext) -> AppResult<ProjectDto> {
        let mut project = self.transfer.import_from(Path::new(request.file.trim()))?;
        if let Some(raw) = request.root.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            project.relink(existing_folder(self.fs.as_ref(), raw)?);
        }
        project.set_active(false);
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
                self.writer.save_checked(snap, &project, true)?;
                Ok(project)
            })
            .await?;
        self.writer.changed(&project.id);
        self.writer.finish(&project, ctx).await
    }
}

/// Address suggestions for the project editor and the create screen, so front-ends never
/// re-derive the default-address rules or guess a free port.
pub struct SuggestAddresses {
    allocator: Arc<LoopbackPortAllocator>,
    settings: Arc<dyn SettingsRepository>,
}

impl SuggestAddresses {
    pub fn new(allocator: Arc<LoopbackPortAllocator>, settings: Arc<dyn SettingsRepository>) -> Self {
        Self { allocator, settings }
    }

    /// `project_id` may be a project that does not exist yet (`DetectionDto::suggested_id`).
    pub async fn execute(&self, project_id: &str) -> AppResult<AddressSuggestionsDto> {
        let id = parse_project_id(project_id)?;
        let settings = self.settings.load()?;
        let standard = SiteAddress::defaults_for(id.as_str(), &settings.tld)?;
        let hostname = standard.iter().find(|a| a.needs_hosts_entry()).cloned().ok_or_else(|| AppError::internal("no hostname in the standard addresses"))?;
        let localhost = SiteAddress::localhost(id.as_str())?;
        let loopback = SiteAddress::loopback_port(self.allocator.peek().await?)?;
        Ok(AddressSuggestionsDto {
            hostname: address_to_dto(&hostname),
            localhost: address_to_dto(&localhost),
            loopback_port: address_to_dto(&loopback),
            no_elevation: vec![address_to_dto(&localhost), address_to_dto(&loopback)],
            standard: standard.iter().map(address_to_dto).collect(),
        })
    }
}
