use std::path::Path;
use std::sync::Arc;

use lampo_domain::mysql::DatabaseName;
use lampo_domain::ports::{DatabaseAdmin, DumpImportOptions, InstalledPackageRegistry, MysqlEndpoint, ProgressSink, ProjectRepository, SettingsRepository, WorktreeRepository};
use lampo_domain::progress::CancelFlag;
use lampo_domain::project::Project;
use lampo_domain::PortError;

use super::service_leases::ServiceLease;
use super::services::ServiceEnsurer;
use super::support::{load_site, mysql_endpoint, validation};
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};

/// A site's MySQL instance (started on demand, leased while the target lives) and its own
/// database.
struct Target {
    endpoint: MysqlEndpoint,
    site_database: Option<DatabaseName>,
    /// Keeps the instance running (no sync restarts it) while the operation uses it.
    _lease: ServiceLease,
}

/// `lampo db …` on the MySQL instance of a site (project or worktree environment). Runs in the
/// app (which owns the MySQL process and starts it on demand).
pub struct DatabaseService {
    admin: Arc<dyn DatabaseAdmin>,
    projects: Arc<dyn ProjectRepository>,
    worktrees: Arc<dyn WorktreeRepository>,
    settings: Arc<dyn SettingsRepository>,
    installed: Arc<dyn InstalledPackageRegistry>,
    services: Arc<dyn ServiceEnsurer>,
}

impl DatabaseService {
    pub fn new(
        admin: Arc<dyn DatabaseAdmin>,
        projects: Arc<dyn ProjectRepository>,
        worktrees: Arc<dyn WorktreeRepository>,
        settings: Arc<dyn SettingsRepository>,
        installed: Arc<dyn InstalledPackageRegistry>,
        services: Arc<dyn ServiceEnsurer>,
    ) -> Self {
        Self { admin, projects, worktrees, settings, installed, services }
    }

    /// Resolves the site's instance and makes sure it runs (`Validation` if it uses no MySQL).
    async fn target(&self, site: &str) -> AppResult<Target> {
        let (site_id, project, worktree) = load_site(self.projects.as_ref(), self.worktrees.as_ref(), site)?;
        let series = project.environment.mysql().ok_or_else(|| validation(format!("`{site_id}` does not use MySQL")))?;
        let endpoint = mysql_endpoint(self.installed.as_ref(), &self.settings.load()?, series)?;
        let lease = self.services.ensure_running(series.service_id()).await?;
        let site_database = match &worktree {
            Some(w) => w.database_name(&project).cloned(),
            None => project.database().cloned(),
        };
        Ok(Target { endpoint, site_database, _lease: lease })
    }

    /// `name`, or the site's own database.
    fn name_or_default(name: Option<&str>, target: &Target) -> AppResult<DatabaseName> {
        match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => Ok(DatabaseName::new(n)?),
            None => target.site_database.clone().ok_or_else(|| validation("the site has no database: pass a database name")),
        }
    }

    fn file(raw: &str) -> AppResult<&Path> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(validation("an SQL file is required"));
        }
        Ok(Path::new(trimmed))
    }

    /// `Validation` if the site uses no MySQL.
    pub async fn list(&self, request: &DatabaseListRequest) -> AppResult<DatabaseListDto> {
        let target = self.target(&request.site).await?;
        let databases = self.admin.list(&target.endpoint).await?;
        let e = &target.endpoint;
        Ok(DatabaseListDto {
            mysql_version: e.series.to_string(),
            host: "127.0.0.1".into(),
            port: e.port,
            user: e.user.clone(),
            databases,
            site_database: target.site_database.map(|d| d.to_string()),
        })
    }

    /// Default name: the site's database.
    pub async fn create(&self, request: &DatabaseRequest) -> AppResult<String> {
        let target = self.target(&request.site).await?;
        let name = Self::name_or_default(request.name.as_deref(), &target)?;
        self.admin.create(&target.endpoint, &name).await?;
        Ok(name.to_string())
    }

    /// Creates a new project's own database on its instance (started on demand). Never fails:
    /// `None` when the project uses no MySQL (or has no database), otherwise the outcome. An
    /// existing database is kept (`already-exists`); a missing MySQL package is reported, never
    /// installed.
    pub async fn provision_project(&self, project: &Project) -> Option<DatabaseSetupDto> {
        let (series, name) = (project.environment.mysql()?, project.database().cloned()?);
        let outcome = |status, message: Option<String>| Some(DatabaseSetupDto { name: name.to_string(), status, message });
        let endpoint = match self.settings.load().map_err(AppError::from).and_then(|s| mysql_endpoint(self.installed.as_ref(), &s, series)) {
            Ok(endpoint) => endpoint,
            Err(e) => {
                tracing::warn!("database `{name}` of the new project `{}` not created: {e}", project.id);
                let status = if e.code == ErrorCode::PackageMissing { DatabaseSetupStatusDto::PackageMissing } else { DatabaseSetupStatusDto::Failed };
                return outcome(status, Some(e.message));
            }
        };
        let created = match self.services.ensure_running(series.service_id()).await {
            Ok(_lease) => match self.admin.create(&endpoint, &name).await {
                Ok(()) => Ok(DatabaseSetupStatusDto::Created),
                Err(PortError::AlreadyExists { .. }) => Ok(DatabaseSetupStatusDto::AlreadyExists),
                Err(e) => Err(AppError::from(e)),
            },
            Err(e) => Err(e),
        };
        match created {
            Ok(status) => outcome(status, None),
            Err(e) => {
                tracing::warn!("could not create the database `{name}` of the new project `{}`: {e}", project.id);
                outcome(DatabaseSetupStatusDto::Failed, Some(e.message))
            }
        }
    }

    /// `name` is required for drop (no implicit default).
    pub async fn drop_database(&self, request: &DatabaseRequest) -> AppResult<String> {
        let raw = request.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| validation("the name of the database to drop is required"))?;
        let name = DatabaseName::new(raw)?;
        let target = self.target(&request.site).await?;
        self.admin.drop_database(&target.endpoint, &name).await?;
        Ok(name.to_string())
    }

    /// A dump that switches database is refused unless `allow_database_switch`. A failed or
    /// cancelled import that already ran statements says the database may be incomplete.
    pub async fn import(&self, request: &DatabaseImportRequest, progress: &dyn ProgressSink, cancel: &CancelFlag) -> AppResult<()> {
        let file = Self::file(&request.file)?;
        let target = self.target(&request.site).await?;
        let name = Self::name_or_default(request.name.as_deref(), &target)?;
        cancel.check()?;
        let options = DumpImportOptions { allow_database_switch: request.allow_database_switch };
        self.admin.import(&target.endpoint, &name, file, options, progress, cancel).await.map_err(|e| match e {
            PortError::Interrupted(cause) => incomplete_import(&name, *cause),
            e => e.into(),
        })
    }

    pub async fn export(&self, request: &DatabaseFileRequest, progress: &dyn ProgressSink, cancel: &CancelFlag) -> AppResult<()> {
        let file = Self::file(&request.file)?;
        let target = self.target(&request.site).await?;
        let name = Self::name_or_default(request.name.as_deref(), &target)?;
        cancel.check()?;
        Ok(self.admin.export(&target.endpoint, &name, file, progress, cancel).await?)
    }
}

/// The error of an import that stopped part-way (same category as `cause`), telling that `name`
/// may be incomplete and how to get back to a consistent state.
fn incomplete_import(name: &DatabaseName, cause: PortError) -> AppError {
    let error = AppError::from(cause);
    let message = format!(
        "{}. The import stopped part-way: the database `{name}` may now be incomplete (the statements that already ran were kept). \
         To restore it, drop it (`lampo db drop {name} --yes`) and import the dump again, or import a backup of it.",
        error.message
    );
    AppError { message, ..error }
}
