//! `defender exclusion add|remove|commands [--php] [--mysql] [--projects [<folder>]]`: runs in this
//! process (the UAC prompt belongs to the terminal), never in the app, which owns no state of it.

use lampo_application::dto::{DefenderExclusionRequestDto, ScanExclusionChangeDto, ScanExclusionOutcomeDto};
use lampo_application::{AppResult, CallContext};

use super::Cmd;
use crate::cli::{DefenderCommand, ExclusionCommand, ExclusionTargets};
use crate::exit_code::ExitCode;
use crate::render;

pub async fn run(c: &Cmd<'_>, command: DefenderCommand) -> AppResult<i32> {
    match command {
        DefenderCommand::Exclusion(command) => exclusion(c, command).await,
    }
}

async fn exclusion(c: &Cmd<'_>, command: ExclusionCommand) -> AppResult<i32> {
    let (change, targets) = match command {
        ExclusionCommand::Add(targets) => (ScanExclusionChangeDto::Add, targets),
        ExclusionCommand::Remove(targets) => (ScanExclusionChangeDto::Remove, targets),
        ExclusionCommand::Commands(targets) => return commands(c, targets.or_php()),
    };
    let result = c.ctx.scan_exclusion.change(request(c, change, targets), CallContext::cli(c.g.allow_elevation)).await?;
    c.io.emit(&result, render::defender_exclusion);
    Ok(exit_code(result.outcome).code())
}

/// The `Add-MpPreference` lines of what `add` would exclude: no prompt, nothing changed.
fn commands(c: &Cmd<'_>, targets: ExclusionTargets) -> AppResult<i32> {
    let commands = c.ctx.scan_exclusion.commands(request(c, ScanExclusionChangeDto::Add, targets))?;
    c.io.emit(&commands, |commands| render::defender_commands(commands));
    Ok(ExitCode::Success.code())
}

/// A relative projects folder is the terminal's: made absolute here, where the cwd is known.
fn request(c: &Cmd<'_>, change: ScanExclusionChangeDto, targets: ExclusionTargets) -> DefenderExclusionRequestDto {
    DefenderExclusionRequestDto {
        projects: targets.projects.is_some(),
        projects_folder: targets.projects.flatten().map(|folder| c.ctx.env.absolute(&folder)),
        ..DefenderExclusionRequestDto::new(change, targets.php, targets.mysql)
    }
}

/// Declined prompt: as for the hosts file. Defender not doing it: an external program failed.
pub(super) fn exit_code(outcome: ScanExclusionOutcomeDto) -> ExitCode {
    match outcome {
        ScanExclusionOutcomeDto::Applied => ExitCode::Success,
        ScanExclusionOutcomeDto::Cancelled => ExitCode::ElevationDenied,
        ScanExclusionOutcomeDto::DefenderUnavailable | ScanExclusionOutcomeDto::Refused | ScanExclusionOutcomeDto::NotConfirmed => ExitCode::External,
    }
}
