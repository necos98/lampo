//! Project CRUD and lifecycle. Config only: these never touch the project folder or `.env`.
//!
//! Every use case that saves or removes a project does it through [`ProjectWriter`] (under the
//! shared allocator lock, after the environment policy and the conflict checks): the rules
//! cannot be skipped by forgetting an optional builder.

use std::sync::Arc;

use lampo_domain::address::SiteAddress;
use lampo_domain::detection::{DetectedApp, combine_hints};
use lampo_domain::events::DomainEvent;
use lampo_domain::mysql::DatabaseName;
use lampo_domain::mysql::accounts::MysqlCredentials;
use lampo_domain::package::PackageKind;
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::{EventPublisher, FileSystemInspector, InstalledPackageRegistry, ProjectDetector, ProjectRepository, SettingsRepository, WorktreeRepository};
use lampo_domain::preset::Preset;
use lampo_domain::project::{DisplayName, Project, ProjectEnvVars, ProjectEnvironment, ProjectId, ProjectRoot, TlsMode, unique_project_id};
use lampo_domain::service::ServiceKind;
use lampo_domain::tcp_port::Port;
use lampo_domain::worktree::WorktreeEnvironment;

use super::allocation::{LoopbackPortAllocator, SiteSnapshot};
use super::database::DatabaseService;
use super::mysql_accounts::ConnectionPasswords;
use super::presets::PresetQueries;
use super::project_policy::require_mysql_for_database;
use super::project_writer::{ProjectWriter, RemovedProject};
use super::support::{load_project, newest_of, package_missing, parse_project_id, project_dto, validation};
use crate::context::{CallContext, ElevationPolicy};
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mapping::{address_from_dto, address_to_dto, addressing_from_dto, addressing_to_dto, credentials_from_dto, environment_from_dto, unreadable_project_to_dto};

/// Preset used when neither the request nor detection names one.
pub const FALLBACK_PRESET: &str = "generic-php";

fn folder_name(root: &ProjectRoot) -> String {
    root.as_str().trim_end_matches('\\').rsplit('\\').next().filter(|s| !s.is_empty() && !s.ends_with(':')).unwrap_or("project").to_string()
}

/// An existing absolute folder, or `Validation`.
pub(super) fn existing_folder(fs: &dyn FileSystemInspector, raw: &str) -> AppResult<ProjectRoot> {
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
    /// The passwords of the connection details shown.
    passwords: Arc<dyn ConnectionPasswords>,
}

impl ProjectQueries {
    pub fn new(
        projects: Arc<dyn ProjectRepository>,
        worktrees: Arc<dyn WorktreeRepository>,
        settings: Arc<dyn SettingsRepository>,
        fs: Arc<dyn FileSystemInspector>,
        passwords: Arc<dyn ConnectionPasswords>,
    ) -> Self {
        Self { projects, worktrees, settings, fs, passwords }
    }

    /// The projects that load, plus the files that do not (so a front-end can say so instead of
    /// silently showing fewer projects).
    pub fn list(&self) -> AppResult<ProjectListDto> {
        let settings = self.settings.load()?;
        let worktrees = self.worktrees.list_all()?;
        let listing = self.projects.scan()?;
        Ok(ProjectListDto {
            projects: listing.projects.iter().map(|p| project_dto(p, &worktrees, &settings, Some(self.fs.as_ref()), self.passwords.as_ref())).collect(),
            unreadable: listing.unreadable.iter().map(unreadable_project_to_dto).collect(),
        })
    }

    /// The compact rows, plus the files that do not load (like [`Self::list`]).
    pub fn summaries(&self) -> AppResult<ProjectSummaryListDto> {
        let settings = self.settings.load()?;
        let worktrees = self.worktrees.list_all()?;
        let listing = self.projects.scan()?;
        let projects = listing
            .projects
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
            .collect();
        Ok(ProjectSummaryListDto { projects, unreadable: listing.unreadable.iter().map(unreadable_project_to_dto).collect() })
    }

    /// `NotFound` if unknown.
    pub fn get(&self, project_id: &str) -> AppResult<ProjectDto> {
        let project = load_project(self.projects.as_ref(), project_id)?;
        Ok(project_dto(&project, &self.worktrees.list_for(&project.id)?, &self.settings.load()?, Some(self.fs.as_ref()), self.passwords.as_ref()))
    }

    /// The full view of `project` in the registry `sites` (a commit's, so nothing is read again).
    pub(crate) fn present(&self, project: &Project, sites: &SiteSnapshot) -> ProjectDto {
        project_dto(project, &sites.worktrees, &sites.settings, Some(self.fs.as_ref()), self.passwords.as_ref())
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
        let listing = self.projects.scan()?;
        let name = folder_name(&root);
        Ok(DetectionDto {
            folder: root.to_string(),
            project_type: best.as_ref().map(|h| h.project_type.clone()),
            preset_id: best.as_ref().map(|h| h.preset.to_string()),
            php_version: best.as_ref().and_then(|h| h.php).map(|p| p.to_string()),
            suggested_id: unique_project_id(&listing.projects, &listing.unreadable_ids(), &ProjectId::from_name(&name)).to_string(),
            suggested_name: name,
            services: best.as_ref().map(|h| h.services.iter().map(ToString::to_string).collect()).unwrap_or_default(),
            warnings: best.as_ref().map(|h| h.warnings.clone()).unwrap_or_default(),
            apps: best.as_ref().map(|h| h.apps.iter().map(detected_app_dto).collect()).unwrap_or_default(),
            evidence: best.map(|h| h.evidence).unwrap_or_default(),
            already_registered: listing.projects.iter().find(|p| p.root == root).map(|p| p.id.to_string()),
        })
    }
}

fn detected_app_dto(app: &DetectedApp) -> DetectedAppDto {
    DetectedAppDto {
        service: app.service.clone(),
        folder: app.folder.clone(),
        php_version: app.php.map(|php| php.to_string()),
        extensions: app.extensions.clone(),
        document_root: app.document_root.clone(),
        environment: app.environment.clone(),
        service_hosts: app.service_hosts.clone(),
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
            let known = self.writer.services().descriptor(&kind).is_some_and(|d| !d.versioned());
            if known && !environment.uses(&kind) {
                environment.use_service(kind, None);
            }
        }
    }

    /// Fills omitted fields (see [`CreateProjectRequest`]), allocates a loopback port for
    /// `AddressingDto::NoElevation` (atomically with the save), checks the rules of every save,
    /// saves, publishes `ProjectsChanged`, creates the database (when the project uses MySQL and
    /// `create_database`), syncs. A project with hosts-file names needs elevation
    /// (`ElevationRequired`, nothing created, otherwise), so a retry with `--allow-elevation`
    /// creates it instead of finding a half-done first attempt.
    ///
    /// PHP: the request's `phpVersion`, else the preset's — raised to the detected minimum
    /// (`composer.json` `require.php`, a container image) when the preset's PHP is older than that.
    /// Services: the preset's, plus the detected unversioned ones (`add_detected_services`).
    /// Environment variables: the environment's, plus `envVars` (which win on a same name).
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
        let extra_vars = ProjectEnvVars::from_pairs(request.env_vars.iter().map(|(name, value)| (name.as_str(), value.clone())))?;
        for (name, value) in extra_vars.iter() {
            environment.env_vars.set(name.clone(), value.clone());
        }
        let explicit = request.addresses.as_ref().map(|list| list.iter().map(address_from_dto).collect::<Result<Vec<_>, _>>()).transpose()?;
        let addressing = addressing_from_dto(request.addressing);

        let build = |snap: &SiteSnapshot, port: Option<Port>| -> AppResult<Project> {
            let id = requested_id.clone().unwrap_or_else(|| snap.unique_project_id(&ProjectId::from_name(name.as_str())));
            let addresses = match &explicit {
                Some(list) => list.clone(),
                None => addressing.addresses_for(id.as_str(), &snap.settings.tld, port)?,
            };
            let mut project = Project::new(id, name.clone(), root.clone(), addresses, environment.clone())?;
            project.origin_preset = Some(preset.id.clone());
            project.mysql_credentials = MysqlCredentials::for_new_project(snap.settings.mysql_project_credentials);
            project.set_active(request.activate);
            require_project_hosts_elevation(ctx, &project, "creating", "nothing was created")?;
            Ok(project)
        };
        let saved = match explicit.is_none() && addressing.needs_loopback_port() {
            true => self.writer.insert_with_free_port(|snap, port| build(snap, Some(port))).await?,
            false => self.writer.insert(|snap| build(snap, None)).await?,
        };
        let database_setup = if request.create_database { self.databases.provision_project(&saved.project).await } else { None };
        let project = self.writer.finish(saved, ctx).await?;
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

    /// `Unsupported` for `tls: local-ca` until the local CA exists; `Validation` for a database
    /// name without MySQL. The environment goes through the same rules as `lampo project set`.
    pub async fn execute(&self, request: UpdateProjectRequest, ctx: &CallContext) -> AppResult<ProjectDto> {
        if request.tls == TlsModeDto::LocalCa {
            return Err(AppError::new(ErrorCode::Unsupported, "HTTPS with a local certificate authority is not available yet"));
        }
        let name = DisplayName::new(&request.name)?;
        let addresses = request.addresses.iter().map(address_from_dto).collect::<Result<Vec<_>, _>>()?;
        let environment = environment_from_dto(&request.environment)?;
        let database = request.database_name.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(DatabaseName::new).transpose()?;
        let credentials = request.mysql_credentials.map(credentials_from_dto);
        if database.is_some() {
            require_mysql_for_database(&environment)?;
        }
        let saved = self
            .writer
            .modify(&request.project_id, |project, _| {
                project.name = name;
                project.set_addresses(addresses)?;
                project.set_environment(environment)?;
                if let Some(db) = database {
                    project.set_database_name(db);
                }
                if let Some(credentials) = credentials {
                    project.mysql_credentials = credentials;
                }
                project.tls = TlsMode::Disabled;
                Ok(true)
            })
            .await?;
        self.writer.finish(saved, ctx).await
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

/// [`require_hosts_elevation`] for adding or removing every hosts-file name of `project`
/// (`action` is the verb: "creating", "importing", "removing").
pub(crate) fn require_project_hosts_elevation(ctx: &CallContext, project: &Project, action: &str, outcome: &str) -> AppResult<()> {
    let names: Vec<String> = project.hosts_file_names().map(ToString::to_string).collect();
    require_hosts_elevation(ctx, &names, &format!("{action} `{}`", project.id), outcome)
}

/// Removes the project's config and its worktree environments' configs. Never deletes folders
/// or databases.
pub struct DeleteProject {
    writer: ProjectWriter,
    worktrees: Arc<dyn WorktreeRepository>,
    events: Arc<dyn EventPublisher>,
}

impl DeleteProject {
    pub fn new(writer: ProjectWriter, worktrees: Arc<dyn WorktreeRepository>, events: Arc<dyn EventPublisher>) -> Self {
        Self { writer, worktrees, events }
    }

    /// Active projects are removed too (the sync stops serving them). A project with hosts-file
    /// names needs elevation (`ElevationRequired` with nothing removed otherwise), so a retry
    /// with `--allow-elevation` finds it still there. Runs under the allocator lock, so a
    /// concurrent edit never brings the project back. The result lists the worktree databases
    /// left in MySQL.
    pub async fn execute(&self, project_id: &str, ctx: &CallContext) -> AppResult<DeleteProjectResultDto> {
        let RemovedProject { project, dependents: worktrees, sites } = self
            .writer
            .remove(project_id, |project| require_project_hosts_elevation(ctx, project, "removing", "nothing was removed"), |project, sites| self.remove_worktrees(project, sites))
            .await?;
        if !worktrees.is_empty() {
            self.events.publish(DomainEvent::WorktreesChanged { project: project.id.clone() });
        }
        self.writer.sync(sites, ctx).await?;
        Ok(DeleteProjectResultDto { orphaned_databases: worktrees.iter().filter_map(WorktreeEnvironment::owned_database).map(ToString::to_string).collect() })
    }

    /// Unregisters the worktree environments of the (already removed) `project`. On failure they
    /// are left stale, never served without their parent, for `lampo worktree prune` and
    /// `lampo doctor` to report: the removal itself has happened. `sites` (the registry after the
    /// removal) loses them only when they are gone.
    fn remove_worktrees(&self, project: &Project, sites: &mut SiteSnapshot) -> Vec<WorktreeEnvironment> {
        let environments: Vec<WorktreeEnvironment> = sites.worktrees.iter().filter(|w| w.parent == project.id).cloned().collect();
        if !environments.is_empty() {
            match self.worktrees.delete_all_for(&project.id) {
                Ok(()) => sites.forget_worktrees_of(&project.id),
                Err(e) => tracing::warn!("project `{}` removed, but not its worktree environments (left stale for `lampo worktree prune`): {e}", project.id),
            }
        }
        environments
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
        let saved = self
            .writer
            .modify(&request.project_id, |project, _| {
                let changed = project.active != request.active;
                project.set_active(request.active);
                Ok(changed)
            })
            .await?;
        self.writer.finish(saved, ctx).await
    }
}

/// Switches a project to another PHP minor (`PackageMissing` if not installed, or if one of its
/// add-ons is not installed for it).
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
        require_php_installed(&self.writer, self.installed.as_ref(), &request.project_id, php)?;
        let saved = self
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
        self.writer.finish(saved, ctx).await
    }
}

/// `PackageMissing` unless a build of PHP `php` is installed; `NotFound` first for an unknown
/// project, so a mistyped id is not reported as a missing PHP.
pub(super) fn require_php_installed(writer: &ProjectWriter, installed: &dyn InstalledPackageRegistry, project_id: &str, php: PhpMinor) -> AppResult<()> {
    if newest_of(&installed.list()?, &PackageKind::PHP, Some(php.as_series()), None).is_some() {
        return Ok(());
    }
    writer.load(project_id)?;
    Err(package_missing(format!("PHP {php}")))
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
        let saved = self
            .writer
            .modify(&request.project_id, |project, _| {
                project.relink(root);
                Ok(true)
            })
            .await?;
        self.writer.finish(saved, ctx).await
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
            hosts_only: vec![address_to_dto(&hostname)],
            default_addressing: addressing_to_dto(settings.default_addressing),
        })
    }
}
