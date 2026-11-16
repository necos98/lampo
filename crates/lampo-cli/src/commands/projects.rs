//! `project ...`.

use lampo_application::control::ControlRequest;
use lampo_application::dto::*;
use lampo_application::AppResult;
use lampo_domain::php::PhpMinor;
use serde::Serialize;

use super::{Cmd, addressing, validation};
use crate::cli::{ProjectCommand, ProjectCreateArgs, ProjectNewArgs, ProjectSetArgs, StarterKitArg};
use crate::render;

pub async fn run(c: &Cmd<'_>, command: ProjectCommand) -> AppResult<i32> {
    match command {
        ProjectCommand::List => {
            let list = c.ctx.projects.summaries()?;
            c.io.emit(&list, |l| render::project_summaries(l));
        }
        ProjectCommand::Info { id } => {
            let project = c.ctx.projects.get(&c.project_id(id.as_ref())?)?;
            c.io.emit(&project, render::project);
        }
        ProjectCommand::Detect { path } => {
            let detection = c.ctx.detection.detect(&c.ctx.env.absolute(path.as_deref().unwrap_or(".")))?;
            c.io.emit(&detection, render::detection);
        }
        ProjectCommand::Create(args) => {
            let request = create_request(c, args)?;
            let result: CreateProjectResultDto = c.app(ControlRequest::ProjectCreate(request)).await?;
            c.io.emit(&result, render::created_project);
            if let Some(setup) = result.database_setup.as_ref().filter(|s| s.status.is_problem()) {
                c.io.err_line(&format!("lampo: warning: the database `{}` was not created: {}", setup.name, setup.message.as_deref().unwrap_or("unknown error")));
            }
        }
        ProjectCommand::New(args) => new_project(c, args).await?,
        ProjectCommand::Activate { id } => set_active(c, id, true).await?,
        ProjectCommand::Deactivate { id } => set_active(c, id, false).await?,
        ProjectCommand::SetPhp { php_version, id } => {
            reject_swapped_set_php_args(&php_version, id.as_deref())?;
            let project_id = c.project_id(id.as_ref())?;
            let project: ProjectDto = c.app(ControlRequest::ProjectSetPhp(SetProjectPhpRequest { project_id, php_version })).await?;
            c.io.emit(&project, render::project);
        }
        ProjectCommand::Set(args) => {
            let project_id = c.project_id(args.id.as_ref())?;
            let project: ProjectDto = c.app(ControlRequest::ProjectPatch(patch_request(project_id, *args)?)).await?;
            c.io.emit(&project, render::project);
        }
        ProjectCommand::Rename { name, id } => {
            let project_id = c.project_id(id.as_ref())?;
            let project: ProjectDto = c.app(ControlRequest::ProjectPatch(ProjectPatchRequest { project_id, name: Some(name), ..Default::default() })).await?;
            c.io.emit(&project, render::project);
        }
        ProjectCommand::Remove { id, yes } => remove(c, id, yes).await?,
        ProjectCommand::Import { file, root } => super::project_import::import(c, file, root).await?,
    }
    Ok(0)
}

/// `--json` result of `project remove`: what was unregistered and what was kept.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRemovedDto {
    pub project_id: String,
    pub name: String,
    /// Kept on disk, untouched.
    pub root: String,
    /// Kept in MySQL (never dropped).
    pub database_name: Option<String>,
    pub was_active: bool,
    /// Worktree environments unregistered with it (their folders are kept too).
    pub worktree_environments: usize,
    /// The worktree environments' own databases, kept in MySQL and no longer used by any site.
    pub orphaned_databases: Vec<String>,
}

/// Reads the project (file-based: unknown id = exit 3 without starting the app), then removes it.
async fn remove(c: &Cmd<'_>, id: Option<String>, yes: bool) -> AppResult<()> {
    let project_id = c.project_id(id.as_ref())?;
    if !yes {
        return Err(validation(format!("refusing to remove project `{project_id}` without --yes (lampo never prompts; its folder and databases are kept)")));
    }
    let project = c.ctx.projects.get(&project_id)?;
    // `null` from an app that predates the result.
    let result: Option<DeleteProjectResultDto> = c.app(ControlRequest::ProjectDelete { project_id: project.id.clone() }).await?;
    let removed = ProjectRemovedDto {
        project_id: project.id,
        name: project.name,
        root: project.root,
        database_name: project.database_name,
        was_active: project.active,
        worktree_environments: project.worktrees.len(),
        orphaned_databases: result.unwrap_or_default().orphaned_databases,
    };
    c.io.emit(&removed, |r| {
        let worktrees = if r.worktree_environments > 0 { format!(" with {} worktree environment(s)", r.worktree_environments) } else { String::new() };
        let database = r.database_name.as_deref().map(|db| format!(" and database `{db}`")).unwrap_or_default();
        let orphaned = if r.orphaned_databases.is_empty() {
            String::new()
        } else {
            format!("; worktree databases {} kept in MySQL", r.orphaned_databases.iter().map(|d| format!("`{d}`")).collect::<Vec<_>>().join(", "))
        };
        format!("removed project {} ({}){worktrees} from Lampo; folder {}{database} kept{orphaned}", r.project_id, r.name, r.root)
    });
    Ok(())
}

/// `project set-php shop 7.4`: the id where the version goes. Refused with the corrected command
/// rather than guessed, since an id may itself look like a version; anything else is left to the
/// app's own validation.
fn reject_swapped_set_php_args(php_version: &str, id: Option<&str>) -> AppResult<()> {
    let is_version = |s: &str| PhpMinor::parse(s).is_ok();
    match id {
        Some(id) if !is_version(php_version) && is_version(id) => {
            Err(validation(format!("invalid PHP version `{php_version}`: the version comes first, did you mean: lampo project set-php {} {}?", id.trim(), php_version.trim())))
        }
        _ => Ok(()),
    }
}

/// `project set` flags → a [`ProjectPatchRequest`]. Values are validated by the app; only the
/// command-line syntax (`KEY=VALUE`, `KIND[@VERSION]`) is parsed here.
fn patch_request(project_id: String, args: ProjectSetArgs) -> AppResult<ProjectPatchRequest> {
    let mut ini_set = std::collections::BTreeMap::new();
    for pair in &args.ini {
        let (key, value) = pair.split_once('=').filter(|(k, _)| !k.trim().is_empty()).ok_or_else(|| validation(format!("--ini expects KEY=VALUE, got `{pair}`")))?;
        ini_set.insert(key.trim().to_string(), value.to_string());
    }
    Ok(ProjectPatchRequest {
        project_id,
        php_version: args.php,
        extensions_add: args.ext,
        extensions_remove: args.no_ext,
        addons_add: args.addon,
        addons_remove: args.no_addon,
        ini_set,
        ini_unset: args.no_ini,
        services_add: args.service.iter().map(|s| service_use(s)).collect(),
        services_remove: args.no_service,
        database_name: args.database,
        public_dir: args.public_dir,
        apache_directives: args.apache_directives,
        name: args.name,
        addresses_add: args.address,
        addresses_remove: args.no_address,
    })
}

/// `redis`, `mysql`, `mysql@5.7`, `mysql-5.7` (the service id form `lampo up` uses).
fn service_use(raw: &str) -> ServiceUseDto {
    let raw = raw.trim();
    let split = raw.split_once('@').or_else(|| raw.rsplit_once('-').filter(|(_, v)| v.starts_with(|c: char| c.is_ascii_digit())));
    match split {
        Some((kind, version)) => ServiceUseDto { kind: kind.trim().to_string(), version: Some(version.trim().to_string()) },
        None => ServiceUseDto { kind: raw.to_string(), version: None },
    }
}

/// Addressing from `--addressing` / `--hosts-name`, else the `default-addressing` setting; the
/// database is created unless `--no-database`.
fn create_request(c: &Cmd<'_>, args: ProjectCreateArgs) -> AppResult<CreateProjectRequest> {
    let root = c.ctx.env.absolute(args.path.as_deref().unwrap_or("."));
    Ok(CreateProjectRequest {
        root,
        name: args.name,
        id: args.id,
        preset_id: args.preset,
        php_version: args.php,
        addressing: addressing::for_new_project(c, args.addressing, args.hosts_name)?,
        activate: args.activate,
        create_database: !args.no_database,
        ..Default::default()
    })
}

async fn set_active(c: &Cmd<'_>, id: Option<String>, active: bool) -> AppResult<()> {
    let project_id = c.project_id(id.as_ref())?;
    let project: ProjectDto = c.app(ControlRequest::ProjectSetActive(SetProjectActiveRequest { project_id, active })).await?;
    c.io.emit(&project, render::project);
    Ok(())
}

/// `project new`: scaffold the folder in the app (streaming the tools' output), then register it
/// like `project create` (detection fills the rest). A hosts-file addressing without
/// `--allow-elevation` is refused before anything is set up.
async fn new_project(c: &Cmd<'_>, args: ProjectNewArgs) -> AppResult<()> {
    let parent_dir = c.ctx.env.absolute(args.parent.as_deref().unwrap_or("."));
    let (folder_name, source) = match (&args.git, &args.laravel) {
        (Some(url), _) => (args.folder.clone(), ScaffoldSourceDto::GitClone { url: url.clone(), branch: args.branch.clone() }),
        (None, Some(name)) => (
            Some(name.clone()),
            ScaffoldSourceDto::Laravel {
                php_version: args.php.clone(),
                starter_kit: match args.starter_kit {
                    StarterKitArg::None => StarterKitDto::None,
                    StarterKitArg::React => StarterKitDto::React,
                    StarterKitArg::Vue => StarterKitDto::Vue,
                    StarterKitArg::Livewire => StarterKitDto::Livewire,
                    StarterKitArg::Svelte => StarterKitDto::Svelte,
                },
                testing: if args.phpunit { TestingFrameworkDto::Phpunit } else { TestingFrameworkDto::Pest },
                git: args.init_git,
                npm: args.npm,
            },
        ),
        (None, None) => unreachable!("clap requires --git or --laravel"),
    };
    let registration = match args.no_register {
        true => None,
        false => Some(addressing::for_new_project(c, args.addressing, false)?),
    };
    if let Some(chosen) = registration {
        addressing::require_elevation_up_front(c, chosen)?;
    }
    let scaffold: ScaffoldProjectResultDto = c.app(ControlRequest::ProjectScaffold(ScaffoldProjectRequest { parent_dir, folder_name, source })).await?;
    let Some(chosen) = registration else {
        c.io.emit(&scaffold, |s| format!("created {}\n", s.folder));
        return Ok(());
    };
    let request = CreateProjectRequest {
        root: scaffold.folder.clone(),
        php_version: args.php.or(scaffold.php_version),
        addressing: chosen,
        activate: args.activate,
        create_database: !args.no_database,
        ..Default::default()
    };
    let result: CreateProjectResultDto = c.app(ControlRequest::ProjectCreate(request)).await?;
    c.io.emit(&result, render::created_project);
    if let Some(setup) = result.database_setup.as_ref().filter(|s| s.status.is_problem()) {
        c.io.err_line(&format!("lampo: warning: the database `{}` was not created: {}", setup.name, setup.message.as_deref().unwrap_or("unknown error")));
    }
    Ok(())
}
