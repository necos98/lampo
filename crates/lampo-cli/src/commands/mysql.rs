//! `mysql defender-exclusion add|remove`: runs in this process (the UAC prompt belongs to the
//! terminal), never in the app, which owns no state of it.

use lampo_application::dto::{ScanExclusionChangeDto, ScanExclusionOutcomeDto};
use lampo_application::{AppResult, CallContext};

use super::Cmd;
use crate::cli::{DefenderExclusionCommand, MysqlCommand};
use crate::exit_code::ExitCode;
use crate::render;

pub async fn run(c: &Cmd<'_>, command: MysqlCommand) -> AppResult<i32> {
    match command {
        MysqlCommand::DefenderExclusion(command) => defender_exclusion(c, command).await,
    }
}

async fn defender_exclusion(c: &Cmd<'_>, command: DefenderExclusionCommand) -> AppResult<i32> {
    let change = match command {
        DefenderExclusionCommand::Add => ScanExclusionChangeDto::Add,
        DefenderExclusionCommand::Remove => ScanExclusionChangeDto::Remove,
    };
    let result = c.ctx.scan_exclusion.change(change, CallContext::cli(c.g.allow_elevation)).await?;
    c.io.emit(&result, render::scan_exclusion);
    Ok(exit_code(result.outcome).code())
}

/// Declined prompt: as for the hosts file. Defender not doing it: an external program failed.
fn exit_code(outcome: ScanExclusionOutcomeDto) -> ExitCode {
    match outcome {
        ScanExclusionOutcomeDto::Applied => ExitCode::Success,
        ScanExclusionOutcomeDto::Cancelled => ExitCode::ElevationDenied,
        ScanExclusionOutcomeDto::DefenderUnavailable | ScanExclusionOutcomeDto::Refused | ScanExclusionOutcomeDto::NotConfirmed => ExitCode::External,
    }
}
