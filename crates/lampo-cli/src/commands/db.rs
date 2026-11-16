//! `db ...`: always through the app (it owns MySQL and starts it on demand).

use lampo_application::control::ControlRequest;
use lampo_application::dto::*;
use lampo_application::AppResult;
use serde::Serialize;

use super::{Cmd, validation};
use crate::cli::DbCommand;
use crate::render;

/// `--json` result of the commands that return nothing useful: what was done to what.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseActionDto {
    pub site: String,
    /// `created`, `dropped`, `imported`, `exported`.
    pub action: &'static str,
    pub database: Option<String>,
    pub file: Option<String>,
}

pub async fn run(c: &Cmd<'_>, command: DbCommand) -> AppResult<i32> {
    match command {
        DbCommand::Drop { yes: false, name } => {
            return Err(validation(format!("refusing to drop database `{name}` without --yes (lampo never prompts)")));
        }
        DbCommand::List => {
            let site = c.site_id()?;
            let list: DatabaseListDto = c.app(ControlRequest::DbList(DatabaseListRequest { site })).await?;
            c.io.emit(&list, render::databases);
        }
        DbCommand::Create { name } => {
            let site = c.site_id()?;
            let created: String = c.app(ControlRequest::DbCreate(DatabaseRequest { site: site.clone(), name })).await?;
            report(c, DatabaseActionDto { site, action: "created", database: Some(created), file: None });
        }
        DbCommand::Drop { name, yes: true } => {
            let site = c.site_id()?;
            let dropped: String = c.app(ControlRequest::DbDrop(DatabaseRequest { site: site.clone(), name: Some(name) })).await?;
            report(c, DatabaseActionDto { site, action: "dropped", database: Some(dropped), file: None });
        }
        DbCommand::Import { file, name, allow_use } => {
            let site = c.site_id()?;
            let file = c.ctx.env.absolute(&file);
            let request = DatabaseImportRequest { site: site.clone(), name: name.clone(), file: file.clone(), allow_database_switch: allow_use };
            let _: serde_json::Value = c.app(ControlRequest::DbImport(request)).await?;
            report(c, DatabaseActionDto { site, action: "imported", database: name, file: Some(file) });
        }
        DbCommand::Export { file, name } => {
            let site = c.site_id()?;
            let file = c.ctx.env.absolute(&file);
            let _: serde_json::Value = c.app(ControlRequest::DbExport(DatabaseFileRequest { site: site.clone(), name: name.clone(), file: file.clone() })).await?;
            report(c, DatabaseActionDto { site, action: "exported", database: name, file: Some(file) });
        }
    }
    Ok(0)
}

fn report(c: &Cmd<'_>, done: DatabaseActionDto) {
    c.io.emit(&done, |d| match (&d.database, &d.file) {
        (Some(db), Some(file)) => format!("{} database {db} ({file})", d.action),
        (None, Some(file)) => format!("{} the site's database ({file})", d.action),
        (Some(db), None) => format!("{} database {db}", d.action),
        (None, None) => d.action.to_string(),
    });
}
