//! `project ...`.

use lampo_application::control::ControlRequest;
use lampo_application::dto::*;
use lampo_application::AppResult;

use super::Cmd;
use crate::cli::{ProjectCommand, ProjectCreateArgs, ProjectNewArgs, StarterKitArg};
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
            let request = create_request(c, args);
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
            let project_id = c.project_id(id.as_ref())?;
            let project: ProjectDto = c.app(ControlRequest::ProjectSetPhp(SetProjectPhpRequest { project_id, php_version })).await?;
            c.io.emit(&project, render::project);
        }
    }
    Ok(0)
}

/// Localhost addressing (no UAC) unless `--hosts-name`; the database is created unless
/// `--no-database`.
fn create_request(c: &Cmd<'_>, args: ProjectCreateArgs) -> CreateProjectRequest {
    let root = c.ctx.env.absolute(args.path.as_deref().unwrap_or("."));
    CreateProjectRequest {
        root,
        name: args.name,
        id: args.id,
        preset_id: args.preset,
        php_version: args.php,
        addressing: if args.hosts_name { AddressingDto::Standard } else { AddressingDto::NoElevation },
        activate: args.activate,
        create_database: !args.no_database,
        ..Default::default()
    }
}

async fn set_active(c: &Cmd<'_>, id: Option<String>, active: bool) -> AppResult<()> {
    let project_id = c.project_id(id.as_ref())?;
    let project: ProjectDto = c.app(ControlRequest::ProjectSetActive(SetProjectActiveRequest { project_id, active })).await?;
    c.io.emit(&project, render::project);
    Ok(())
}

/// `project new`: scaffold the folder in the app (streaming the tools' output), then register it
/// like `project create` (detection fills the rest).
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
    let scaffold: ScaffoldProjectResultDto = c.app(ControlRequest::ProjectScaffold(ScaffoldProjectRequest { parent_dir, folder_name, source })).await?;
    if args.no_register {
        c.io.emit(&scaffold, |s| format!("created {}\n", s.folder));
        return Ok(());
    }
    let request = CreateProjectRequest {
        root: scaffold.folder.clone(),
        php_version: args.php.or(scaffold.php_version),
        addressing: AddressingDto::NoElevation,
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
