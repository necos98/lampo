//! `worktree ...`.

use lampo_application::control::ControlRequest;
use lampo_application::dto::*;
use lampo_application::AppResult;
use lampo_domain::site::SiteId;

use super::{Cmd, validation};
use crate::cli::{WorktreeCommand, WorktreeDownArgs, WorktreePruneArgs, WorktreeUpArgs};
use crate::render;

pub async fn run(c: &Cmd<'_>, command: WorktreeCommand) -> AppResult<i32> {
    match command {
        WorktreeCommand::Up(args) => up(c, args).await?,
        WorktreeCommand::Down(args) => down(c, args).await?,
        WorktreeCommand::List => {
            let list = c.ctx.worktrees.list(project_filter(c).as_deref())?;
            c.io.emit(&list, |l| render::worktrees(l));
        }
        WorktreeCommand::Prune(args) => prune(c, args).await?,
    }
    Ok(0)
}

/// `--project shop` and `--project shop@fix-1` both narrow `list`/`prune` to project `shop`
/// (parsed with the domain's `SiteId` grammar; anything else is passed on for the use case to
/// reject).
fn project_filter(c: &Cmd<'_>) -> Option<String> {
    c.g.project.as_deref().map(project_of)
}

/// `shared`, `fresh` or `clone:<database>`.
pub fn parse_database_mode(value: &str) -> AppResult<DatabaseModeDto> {
    match value.trim() {
        "shared" => Ok(DatabaseModeDto::Shared),
        "fresh" => Ok(DatabaseModeDto::Fresh),
        other => match other.strip_prefix("clone:") {
            Some(source) if !source.trim().is_empty() => Ok(DatabaseModeDto::Clone { source: source.trim().to_string() }),
            _ => Err(validation(format!("invalid --db `{value}`: use shared, fresh or clone:<database>"))),
        },
    }
}

/// The project part of a site id (`shop@fix-1` → `shop`); unparsable input is returned as is.
fn project_of(raw: &str) -> String {
    match raw.parse::<SiteId>() {
        Ok(site) => site.project().to_string(),
        Err(_) => raw.to_string(),
    }
}

/// A worktree site id (`shop@fix-1`) as opposed to a folder.
pub fn is_site_id(target: &str) -> bool {
    target.parse::<SiteId>().is_ok_and(|site| site.is_worktree())
}

async fn up(c: &Cmd<'_>, args: WorktreeUpArgs) -> AppResult<()> {
    let request =
        WorktreeUpRequest { path: c.ctx.env.absolute(args.path.as_deref().unwrap_or(".")), project_id: args.parent.or_else(|| project_filter(c)), database: parse_database_mode(&args.db)? };
    let result: WorktreeUpResultDto = c.app(ControlRequest::WorktreeUp(request)).await?;
    c.io.emit(&result, render::worktree_up);
    Ok(())
}

async fn down(c: &Cmd<'_>, args: WorktreeDownArgs) -> AppResult<()> {
    let target = match args.target.as_deref() {
        Some(t) if is_site_id(t) => t.to_string(),
        Some(path) => c.ctx.env.absolute(path),
        None => c.ctx.env.absolute("."),
    };
    let result: WorktreeDownResultDto = c.app(ControlRequest::WorktreeDown(WorktreeDownRequest { target, drop_database: args.drop_database })).await?;
    c.io.emit(&result, render::worktree_down);
    Ok(())
}

async fn prune(c: &Cmd<'_>, args: WorktreePruneArgs) -> AppResult<()> {
    let request = PruneWorktreesRequest { project_id: project_filter(c), dry_run: args.dry_run, drop_databases: args.drop_databases };
    let report: PruneReportDto = c.app(ControlRequest::WorktreePrune(request)).await?;
    c.io.emit(&report, render::prune);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_modes() {
        assert_eq!(parse_database_mode("shared").unwrap(), DatabaseModeDto::Shared);
        assert_eq!(parse_database_mode("fresh").unwrap(), DatabaseModeDto::Fresh);
        assert_eq!(parse_database_mode("clone:shop").unwrap(), DatabaseModeDto::Clone { source: "shop".into() });
        assert!(parse_database_mode("clone:").is_err());
        assert!(parse_database_mode("copy").is_err());
    }

    #[test]
    fn site_ids_are_told_from_folders() {
        assert!(is_site_id("shop@feature-x"));
        assert!(!is_site_id(r"C:\work\shop@x"));
        assert!(!is_site_id("../wt"));
        assert!(!is_site_id("shop"));
        assert!(!is_site_id("Shop@X"), "not a valid site id");
        assert_eq!(project_of("shop@fix-1"), "shop");
        assert_eq!(project_of("shop"), "shop");
    }
}
