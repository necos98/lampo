//! Agent-facing queries: one-call context, toolchain environment, exec plan. All file-based.

use std::sync::Arc;

use lampo_domain::logs::LogSource;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{ControlChannelClient, FileSystemInspector, GitInspector, ProjectRepository, SettingsRepository, WorktreeRepository};
use lampo_domain::service::ServiceKind;
use lampo_domain::site::SiteId;

use super::resolution::{SiteResolver, resolution_to_dto};
use super::service_catalog::ServiceCatalog;
use super::mysql_accounts::{ConnectionPasswords, NoConnectionPasswords};
use super::support::{composer_channel_name, shown_connection, validation, worktree_dto};
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mapping::mail_to_dto;

/// `lampo agent context`. Inside a git worktree with an environment: that environment's URL
/// and database; without one: the parent's data plus a hint to run `lampo worktree up`.
pub struct AgentContextQuery {
    resolver: Arc<SiteResolver>,
    projects: Arc<dyn ProjectRepository>,
    worktrees: Arc<dyn WorktreeRepository>,
    settings: Arc<dyn SettingsRepository>,
    git: Arc<dyn GitInspector>,
    fs: Arc<dyn FileSystemInspector>,
    control: Arc<dyn ControlChannelClient>,
    paths: LampoPaths,
    services: Arc<ServiceCatalog>,
    passwords: Arc<dyn ConnectionPasswords>,
}

pub struct AgentContextDeps {
    pub resolver: Arc<SiteResolver>,
    pub projects: Arc<dyn ProjectRepository>,
    pub worktrees: Arc<dyn WorktreeRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub git: Arc<dyn GitInspector>,
    pub fs: Arc<dyn FileSystemInspector>,
    pub control: Arc<dyn ControlChannelClient>,
    pub paths: LampoPaths,
    pub services: Arc<ServiceCatalog>,
    /// The password of the database connection shown.
    pub passwords: Arc<dyn ConnectionPasswords>,
}

impl AgentContextQuery {
    pub fn new(deps: AgentContextDeps) -> Self {
        let AgentContextDeps { resolver, projects, worktrees, settings, git, fs, control, paths, services, passwords } = deps;
        Self { resolver, projects, worktrees, settings, git, fs, control, paths, services, passwords }
    }

    /// `ProjectNotResolved` when nothing matched and there is no default PHP.
    pub fn execute(&self, request: &ResolveRequestDto) -> AppResult<AgentContextDto> {
        let resolution = self.resolver.resolve(request)?.ok_or_else(|| AppError::new(ErrorCode::ProjectNotResolved, format!("no project for `{}` and no default PHP", request.cwd)))?;
        let resolved = resolution_to_dto(&resolution);
        let app_running = self.control.is_server_running();
        let settings = self.settings.load()?;
        let target = &resolution.target;
        let mut hints = Vec::new();

        let php = AgentPhpDto {
            version: target.php.to_string(),
            binary: target.php_exe.display().to_string(),
            ini_dir: target.phprc_dir.as_ref().map(|d| d.display().to_string()),
            composer_phar: target.composer_phar.as_ref().map(|d| d.display().to_string()),
            composer_channel: composer_channel_name(target.composer_channel),
        };
        if target.composer_phar.is_none() {
            hints.push(format!("Composer for PHP {} is not installed: `lampo package install composer`", target.php));
        }

        let mut ctx = AgentContextDto {
            schema_version: AGENT_CONTEXT_SCHEMA_VERSION,
            app_running,
            resolution: resolved.clone(),
            project_id: None,
            project_name: None,
            root: None,
            worktree: None,
            url: None,
            urls: Vec::new(),
            document_root: None,
            php,
            database: None,
            mail: None,
            services: Vec::new(),
            logs: Vec::new(),
            commands: cli_command_catalog(),
            hints: Vec::new(),
        };

        // The git worktree the cwd is in, if it is a linked one.
        let checkout = self.git.inspect(&request.cwd).ok().flatten().filter(|c| c.main_root.is_some());

        if let Some(site) = &resolution.site {
            let project = self.projects.get(site.project())?.ok_or_else(|| AppError::not_found("project", site.project()))?;
            let worktree = match site {
                SiteId::Worktree { parent, slug } => self.worktrees.get(parent, slug)?,
                SiteId::Project(_) => None,
            };
            ctx.project_id = Some(project.id.to_string());
            ctx.project_name = Some(project.name.to_string());
            match &worktree {
                Some(w) => {
                    ctx.root = Some(w.root.to_string());
                    ctx.url = Some(w.url());
                    ctx.urls = worktree_dto(w, &project, &settings, None, &NoConnectionPasswords).urls;
                    ctx.document_root = Some(w.document_root(&project).display().to_string());
                    ctx.database = shown_connection(w.database_connection(&project, &settings).ok().flatten(), self.passwords.as_ref());
                    ctx.worktree = Some(AgentWorktreeDto { worktree_root: w.root.to_string(), branch: w.branch.clone(), environment_up: true, site: Some(site.to_string()) });
                    if !self.fs.is_dir(w.root.as_path()) {
                        hints.push(format!("the folder of `{site}` no longer exists: `lampo worktree prune`"));
                    }
                }
                None => {
                    ctx.root = Some(project.root.to_string());
                    ctx.url = Some(project.url(&settings));
                    ctx.urls = project.urls(&settings);
                    ctx.document_root = Some(project.document_root().display().to_string());
                    ctx.database = shown_connection(project.database_connection(&settings).ok().flatten(), self.passwords.as_ref());
                    if !project.active {
                        hints.push(format!("project `{}` is not active, so it is not served: `lampo project activate {}`", project.id, project.id));
                    }
                }
            }
            if resolved.kind == ResolvedKind::WorktreeWithoutEnvironment {
                let c = checkout.as_ref();
                ctx.worktree = Some(AgentWorktreeDto {
                    worktree_root: c.map(|c| c.worktree_root.to_string()).or(resolved.worktree_root.clone()).unwrap_or_default(),
                    branch: c.and_then(|c| c.branch.clone()),
                    environment_up: false,
                    site: None,
                });
                hints.push(format!(
                    "no worktree environment is up for this worktree: run `lampo worktree up` to get its own URL (and optionally database); until then the URL and database shown are those of `{}`",
                    project.id
                ));
            }
            if project.environment.mysql().is_some() && project.mysql_project_account().is_none() {
                hints.push(format!(
                    "project `{}` connects to MySQL as root (kept for projects created before dedicated accounts): `lampo project set {} --mysql-credentials project` gives it an account of its own, then update DB_USERNAME/DB_PASSWORD in its .env",
                    project.id, project.id
                ));
            }
            if project.environment.uses(&ServiceKind::MAILPIT) {
                ctx.mail = Some(mail_to_dto(&settings.mail_connection()));
            }
            ctx.services = project
                .environment
                .service_ids()
                .filter_map(|id| {
                    let port = self.services.port(&id, &settings)?;
                    Some(ServiceEndpointDto {
                        service: id.to_string(),
                        kind: id.kind().to_string(),
                        label: self.services.label(&id),
                        host: "127.0.0.1".into(),
                        port: port.get(),
                        ui_url: self.services.ui_url(&id, &settings),
                    })
                })
                .collect();
            ctx.logs = LogSource::for_site(site, self.services.with_error_log(project.environment.service_ids()))
                .iter()
                .map(|s| {
                    let file = s.file(&self.paths);
                    let exists = file.parent().is_some_and(|dir| self.fs.is_dir(dir));
                    LogSourceDto { id: s.to_string(), file: file.display().to_string(), exists }
                })
                .collect();
        } else {
            hints.push("no project is registered for this folder (the global default PHP is used): `lampo project create`".into());
        }
        if !app_running {
            hints.push("the Lampo app is not running: mutating commands start it automatically".into());
        }
        ctx.hints = hints;
        Ok(ctx)
    }
}

/// The catalog of CLI commands advertised to agents (`AgentContextDto::commands`).
/// Keep in sync with `lampo-cli`'s clap definitions.
pub fn cli_command_catalog() -> Vec<CliCommandDto> {
    let c = |name: &str, usage: &str, description: &str, mutating: bool, requires_app: bool| CliCommandDto {
        name: name.into(),
        usage: usage.into(),
        description: description.into(),
        mutating,
        requires_app,
    };
    vec![
        c("lampo status", "lampo status [--json]", "Services and projects overview.", false, false),
        c("lampo agent context", "lampo agent context [--json]", "Everything about the current project/worktree in one call.", false, false),
        c("lampo env", "lampo env [--json|--shell powershell|cmd|bash]", "Print the toolchain environment (PATH, PHPRC, LAMPO_*).", false, false),
        c("lampo exec", "lampo exec -- <command> [args...]", "Run a command with the project's PHP/Composer/MySQL on PATH.", false, false),
        c(
            "lampo which",
            "lampo which <php|composer|mysql|mysqldump|redis-cli> [--json]",
            "Which site and binary the shim of a tool would use here, and why (--project, LAMPO_PROJECT, folder, git worktree, global default).",
            false,
            false,
        ),
        c("lampo completions", "lampo completions <powershell|bash|zsh|fish>", "Print the shell completion script for lampo.", false, false),
        c(
            "lampo open",
            "lampo open [--code|--explorer|--terminal|--browser] [--json]",
            "Open the current site in VS Code, Explorer (default), a PowerShell console with its toolchain, or its URL in the browser; returns at once.",
            false,
            false,
        ),
        c("lampo project list", "lampo project list [--json]", "List projects.", false, false),
        c("lampo project info", "lampo project info [<id>] [--json]", "Show one project.", false, false),
        c(
            "lampo project detect",
            "lampo project detect [<path>] [--json]",
            "Show what Lampo detects in a folder (type, preset, PHP, services, warnings) without registering it.",
            false,
            false,
        ),
        c(
            "lampo project create",
            "lampo project create [<path>] [--name] [--preset] [--php] [--addressing hosts-only|standard|no-elevation] [--activate] [--no-database] [--env <key>=<value>]",
            "Register a folder as a project and create its MySQL database (unless --no-database).",
            true,
            true,
        ),
        c(
            "lampo project new",
            "lampo project new (--git <url> [--branch] [--folder] | --laravel <name> [--php] [--starter-kit] [--phpunit] [--init-git] [--npm]) [--in <dir>] [--addressing <mode>] [--activate] [--no-database] [--no-register]",
            "Clone a repository or create a Laravel app (Lampo-managed installer), then register it.",
            true,
            true,
        ),
        c("lampo project activate", "lampo project activate [<id>]", "Activate a project (serve it).", true, true),
        c("lampo project deactivate", "lampo project deactivate [<id>]", "Deactivate a project.", true, true),
        c("lampo project set-php", "lampo project set-php <minor> [<id>]", "Switch the project's PHP minor.", true, true),
        c(
            "lampo project set",
            "lampo project set [<id>] [--ext <name>] [--no-ext <name>] [--addon xdebug|php-redis] [--no-addon <kind>] [--ini <key>=<value>] [--no-ini <key>] [--env <key>=<value>] [--no-env <key>] [--service redis|mailpit|mysql[@<series>]] [--no-service <kind>] [--database <name>] [--public-dir <dir>] [--apache-directives <text>] [--name <name>] [--address <addr>] [--no-address <addr>]",
            "Change the project's extensions, add-ons, php.ini values, environment variables (web and CLI), services, database name, public folder, Apache directives, display name or addresses (repeatable flags, one atomic edit). Addresses: <name>.localhost, 127.0.0.1:<port>, loopback (free port), or a hosts-file <name>.<tld> (needs --allow-elevation).",
            true,
            true,
        ),
        c("lampo project rename", "lampo project rename <name> [<id>]", "Change the project's display name (id, URLs and database stay).", true, true),
        c("lampo project remove", "lampo project remove [<id>] --yes", "Unregister a project and its worktree environments (folder and databases are kept; ask the user first).", true, true),
        c(
            "lampo project import",
            "lampo project import <file> [--root <dir>]",
            "Register a project exported from Lampo (inactive). Its Apache directives, environment variables and code-running php.ini keys are never imported: they are listed (droppedApacheDirectives, droppedEnvVars by name, droppedPhpIniKeys) for the user to review.",
            true,
            true,
        ),
        c("lampo worktree up", "lampo worktree up [<path>] [--db shared|fresh|clone:<db>]", "Create (or return) the worktree's own environment.", true, true),
        c("lampo worktree down", "lampo worktree down [<path|site>] [--drop-database]", "Remove the worktree environment.", true, true),
        c("lampo worktree list", "lampo worktree list [--project <id>] [--json]", "List worktree environments.", false, false),
        c("lampo worktree prune", "lampo worktree prune [--dry-run] [--drop-databases]", "Remove environments whose folder is gone.", true, true),
        c("lampo up", "lampo up [<service>...]", "Start services (all planned when none given).", true, true),
        c("lampo down", "lampo down [<service>...]", "Stop services.", true, true),
        c("lampo restart", "lampo restart [<service>...]", "Restart services.", true, true),
        c("lampo quit", "lampo quit", "Stop every service and exit the Lampo app (does not start it).", true, false),
        c("lampo logs", "lampo logs [<source>] [--tail N] [--since 10m] [--follow]", "Read Apache/PHP/MySQL logs.", false, false),
        c("lampo package list", "lampo package list [--kind <k>] [--installed] [--json]", "List catalog or installed packages.", false, false),
        c("lampo package install", "lampo package install <kind> [<version|series>]", "Download and install a package.", true, true),
        c(
            "lampo package uninstall",
            "lampo package uninstall <kind> [<version|series>] [--php <minor>] --yes",
            "Remove an installed package (refused while a site, the default PHP or an always-on MySQL needs it). Ask the user first.",
            true,
            true,
        ),
        c(
            "lampo package extensions",
            "lampo package extensions [--php <minor>] [--json]",
            "List the bundled PHP extensions `lampo project set --ext` accepts, flagged as enabled for the current project.",
            false,
            false,
        ),
        c("lampo db list", "lampo db list [--json]", "List databases on the project's MySQL.", false, true),
        c("lampo db create", "lampo db create [<name>]", "Create a database.", true, true),
        c("lampo db drop", "lampo db drop <name> --yes", "Drop a database.", true, true),
        c(
            "lampo db import",
            "lampo db import <file.sql> [<name>] [--allow-use] [--compat mysql80|mariadb]",
            "Import an SQL dump (.sql, .sql.gz or a .zip with one .sql); one with USE / CREATE DATABASE needs --allow-use; one from MySQL 8.0 or MariaDB may need --compat.",
            true,
            true,
        ),
        c("lampo db export", "lampo db export <file.sql> [<name>]", "Export a database (gzipped if the file ends in .gz).", false, true),
        c("lampo doctor", "lampo doctor [--json]", "Diagnose the installation (VC++ runtimes, ports, PATH shadowing…).", false, false),
        c("lampo shims status", "lampo shims status [--json]", "Whether the php/composer/mysql shims are installed, on PATH and not shadowed.", false, false),
        c("lampo shims install", "lampo shims install", "Install the shims and add their folder to the user PATH (no elevation).", true, false),
        c("lampo shims uninstall", "lampo shims uninstall [--json]", "Take the shim folder off the user PATH and delete the shims and the lampo copy in it.", true, false),
        c(
            "lampo settings get",
            "lampo settings [get] [--json]",
            "Show the global settings: Apache, MySQL, Redis and Mailpit (SMTP + web UI) ports, default PHP, loopback port range.",
            false,
            false,
        ),
        c(
            "lampo settings set",
            "lampo settings set [--apache-port <port>] [--mysql-port <series>=<port>] [--redis-port <port>] [--port <service>=<port>] [--ui-port <service>=<port>] [--default-php <minor>] [--loopback-range <start>-<end>] [--default-addressing <mode>]",
            "Change ports (checked for conflicts and use; running services restart on the new port), the default PHP, the loopback range or the default addressing of new projects. Ask the user before changing ports.",
            true,
            true,
        ),
    ]
}

/// `lampo env` / `lampo exec`.
pub struct ToolchainQuery {
    resolver: Arc<SiteResolver>,
    paths: LampoPaths,
}

impl ToolchainQuery {
    pub fn new(resolver: Arc<SiteResolver>, paths: LampoPaths) -> Self {
        Self { resolver, paths }
    }

    /// `ShimTarget::toolchain_env` of the resolved site (`ProjectNotResolved` if none).
    pub fn env(&self, request: &ResolveRequestDto) -> AppResult<ToolchainEnvDto> {
        let r = self.resolver.resolve(request)?.ok_or_else(|| AppError::new(ErrorCode::ProjectNotResolved, format!("no project for `{}` and no default PHP", request.cwd)))?;
        let env = r.target.toolchain_env(r.site.as_ref().zip(r.root.as_ref()), &self.paths.shim_dir());
        Ok(ToolchainEnvDto {
            site: r.site.as_ref().map(|s| s.to_string()),
            path_prepend: env.path_prepend.iter().map(|p| p.display().to_string()).collect(),
            vars: env.vars.into_iter().collect(),
            notice: r.ignored_inherited_notice(),
        })
    }

    /// What `lampo exec` must spawn (`Validation` if `command` is empty).
    pub fn exec_plan(&self, request: &ResolveRequestDto, command: &[String]) -> AppResult<ExecPlanDto> {
        let (program, args) = command.split_first().filter(|(p, _)| !p.trim().is_empty()).ok_or_else(|| validation("no command given: `lampo exec -- <command> [args...]`"))?;
        Ok(ExecPlanDto { program: program.clone(), args: args.to_vec(), env: self.env(request)? })
    }
}
