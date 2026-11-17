//! `mysql defender-exclusion add|remove`: the MySQL-only form of `defender exclusion`, kept with
//! its own output for existing scripts. Runs in this process (the UAC prompt belongs to the
//! terminal), never in the app, which owns no state of it.

use lampo_application::dto::ScanExclusionChangeDto;
use lampo_application::{AppResult, CallContext};

use super::Cmd;
use super::defender::exit_code;
use crate::cli::{DefenderExclusionCommand, MysqlCommand};
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
    let result = c.ctx.scan_exclusion.change_mysql(change, CallContext::cli(c.g.allow_elevation)).await?;
    c.io.emit(&result, render::scan_exclusion);
    Ok(exit_code(result.outcome).code())
}
