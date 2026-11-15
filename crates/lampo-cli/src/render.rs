//! Human (non-JSON) rendering of DTOs: plain text, no colour, no cursor control, so it stays
//! readable when captured by an agent. Pure functions from DTO to `String`.

use lampo_application::dto::*;

use crate::cli::ShellKind;
use crate::exit_code::ExitCode;

/// Left-aligned columns separated by two spaces; the last column is not padded.
pub fn table(rows: &[Vec<String>]) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns).map(|c| rows.iter().filter_map(|r| r.get(c)).map(|s| s.chars().count()).max().unwrap_or(0)).collect();
    rows.iter()
        .map(|row| {
            let last = row.len().saturating_sub(1);
            let cells: Vec<String> = row.iter().enumerate().map(|(i, cell)| if i == last { cell.clone() } else { format!("{cell:<width$}", width = widths[i]) }).collect();
            cells.join("  ").trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn row<const N: usize>(cells: [&str; N]) -> Vec<String> {
    cells.iter().map(|c| c.to_string()).collect()
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn state_name(state: ServiceStateDto) -> &'static str {
    match state {
        ServiceStateDto::Stopped => "stopped",
        ServiceStateDto::Starting => "starting",
        ServiceStateDto::Running => "running",
        ServiceStateDto::Stopping => "stopping",
        ServiceStateDto::Failed => "failed",
        ServiceStateDto::Unknown => "unknown",
    }
}

pub fn status(status: &StatusDto) -> String {
    let mut out = format!("Lampo app: {}\n", if status.app_running { "running" } else { "not running (service states unknown; `lampo up` starts it)" });
    let mut rows = vec![row(["SERVICE", "STATE", "PORT", "PID", "NOTE"])];
    for s in &status.services {
        rows.push(vec![
            s.service.clone(),
            state_name(s.state).into(),
            s.port.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
            s.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
            s.message.clone().unwrap_or_default(),
        ]);
    }
    out.push_str(&table(&rows));
    out.push_str(&format!("\n\nProjects ({}), worktree environments: {}\n", status.projects.len(), status.worktree_count));
    out.push_str(&project_summaries(&status.projects));
    out
}

pub fn project_summaries(projects: &[ProjectSummaryDto]) -> String {
    if projects.is_empty() {
        return "No projects. Register a folder with `lampo project create <path>`.".into();
    }
    let mut rows = vec![row(["ID", "ACTIVE", "PHP", "MYSQL", "WORKTREES", "URL", "ROOT"])];
    for p in projects {
        rows.push(vec![
            p.id.clone(),
            yes_no(p.active).into(),
            p.php_version.clone(),
            p.mysql_version.clone().unwrap_or_else(|| "-".into()),
            p.worktree_count.to_string(),
            p.url.clone(),
            p.root.clone(),
        ]);
    }
    table(&rows)
}

fn database_lines(db: &DatabaseConnectionDto, indent: &str) -> String {
    format!("{indent}mysql {} at {}:{}, user {}, password {:?}, database {}", db.mysql_version, db.host, db.port, db.user, db.password, db.database)
}

/// `project create`: the project, then what happened to its database.
pub fn created_project(r: &CreateProjectResultDto) -> String {
    let mut out = project(&r.project);
    if let Some(setup) = &r.database_setup {
        let status = match setup.status {
            DatabaseSetupStatusDto::Created => "created",
            DatabaseSetupStatusDto::AlreadyExists => "already exists (kept)",
            DatabaseSetupStatusDto::PackageMissing => "skipped, MySQL not installed",
            DatabaseSetupStatusDto::Failed => "failed",
        };
        out.push_str(&format!("  database setup: {status} ({})\n", setup.name));
    }
    out
}

pub fn project(p: &ProjectDto) -> String {
    let mut out = format!("{} ({})\n", p.id, p.name);
    out.push_str(&format!("  root:      {}{}\n", p.root, if p.folder_exists { "" } else { " (folder missing)" }));
    out.push_str(&format!("  active:    {}\n", yes_no(p.active)));
    out.push_str(&format!("  url:       {}\n", p.url));
    if p.urls.len() > 1 {
        out.push_str(&format!("  all urls:  {}\n", p.urls.join(", ")));
    }
    out.push_str(&format!("  docroot:   {}\n", p.document_root));
    out.push_str(&format!("  php:       {}\n", p.environment.php_version));
    if !p.environment.extensions.is_empty() || !p.environment.addons.is_empty() {
        let mut all = p.environment.extensions.clone();
        all.extend(p.environment.addons.iter().cloned());
        out.push_str(&format!("  extensions: {}\n", all.join(", ")));
    }
    match &p.database {
        Some(db) => out.push_str(&format!("  database:\n{}\n", database_lines(db, "    "))),
        None => out.push_str("  database:  none\n"),
    }
    let services: Vec<String> = p.environment.services.iter().map(|s| s.version.as_ref().map(|v| format!("{}-{v}", s.kind)).unwrap_or_else(|| s.kind.clone())).collect();
    out.push_str(&format!("  services:  {}\n", if services.is_empty() { "none".to_string() } else { services.join(", ") }));
    if let Some(preset) = &p.origin_preset {
        out.push_str(&format!("  preset:    {preset}\n"));
    }
    if !p.worktrees.is_empty() {
        out.push_str("  worktrees:\n");
        for w in &p.worktrees {
            out.push_str(&format!("    {}  {}{}\n", w.site, w.url, if w.stale { "  (stale)" } else { "" }));
        }
    }
    out
}

pub fn worktrees(list: &[WorktreeEnvironmentDto]) -> String {
    if list.is_empty() {
        return "No worktree environments. Create one with `lampo worktree up [<path>]`.".into();
    }
    let mut rows = vec![row(["SITE", "BRANCH", "DB", "URL", "ROOT"])];
    for w in list {
        let mode = match &w.database_mode {
            DatabaseModeDto::Shared => "shared".to_string(),
            DatabaseModeDto::Fresh => "fresh".to_string(),
            DatabaseModeDto::Clone { source } => format!("clone:{source}"),
        };
        rows.push(vec![w.site.clone(), w.branch.clone().unwrap_or_else(|| "-".into()), mode, w.url.clone(), format!("{}{}", w.root, if w.stale { " (stale)" } else { "" })]);
    }
    table(&rows)
}

pub fn worktree_up(result: &WorktreeUpResultDto) -> String {
    let w = &result.environment;
    let mut out = format!("{} environment {}\n", if result.created { "Created" } else { "Existing" }, w.site);
    out.push_str(&format!("  url:      {}\n", w.url));
    if w.urls.len() > 1 {
        out.push_str(&format!("  all urls: {}\n", w.urls.join(", ")));
    }
    out.push_str(&format!("  root:     {}\n  docroot:  {}\n", w.root, w.document_root));
    if let Some(db) = &w.database {
        out.push_str(&format!("  database:\n{}\n", database_lines(db, "    ")));
    }
    out
}

pub fn worktree_down(result: &WorktreeDownResultDto) -> String {
    match &result.database_dropped {
        Some(db) => format!("Removed environment {} and dropped database {db}", result.site),
        None => format!("Removed environment {}", result.site),
    }
}

pub fn prune(report: &PruneReportDto) -> String {
    let verb = if report.dry_run { "Would remove" } else { "Removed" };
    if report.removed.is_empty() {
        return "No stale worktree environments.".into();
    }
    let mut out = format!("{verb} {} environment(s): {}", report.removed.len(), report.removed.join(", "));
    if !report.dropped_databases.is_empty() {
        out.push_str(&format!("\n{} databases: {}", if report.dry_run { "Would drop" } else { "Dropped" }, report.dropped_databases.join(", ")));
    }
    out
}

pub fn sync_report(report: &SyncReportDto) -> String {
    let mut lines = Vec::new();
    for (label, items) in [("started", &report.started), ("stopped", &report.stopped), ("restarted", &report.restarted)] {
        if !items.is_empty() {
            lines.push(format!("{label}: {}", items.join(", ")));
        }
    }
    if report.hosts_file_updated {
        lines.push("hosts file updated".into());
    }
    lines.extend(report.warnings.iter().map(|w| format!("warning: {w}")));
    if lines.is_empty() {
        lines.push("Nothing to do.".into());
    }
    lines.join("\n")
}

pub fn log_sources(sources: &[LogSourceDto]) -> String {
    let mut rows = vec![row(["SOURCE", "EXISTS", "FILE"])];
    rows.extend(sources.iter().map(|s| vec![s.id.clone(), yes_no(s.exists).into(), s.file.clone()]));
    table(&rows)
}

pub fn log_lines(lines: &LogLinesDto) -> String {
    lines.lines.iter().map(|l| l.text.trim_end_matches(['\r', '\n'])).collect::<Vec<_>>().join("\n")
}

pub fn catalog(entries: &[CatalogEntryDto]) -> String {
    let mut rows = vec![row(["KIND", "VERSION", "PHP", "CHANNEL", "INSTALLED", "NOTE"])];
    for e in entries {
        let mut notes = Vec::new();
        if e.end_of_life {
            notes.push("end-of-life".to_string());
        }
        if e.vc_runtime_installed == Some(false) {
            notes.push(format!("needs VC++ runtime {}", e.vc_runtime.clone().unwrap_or_default()));
        }
        rows.push(vec![
            e.id.kind.clone(),
            e.id.version.clone(),
            e.id.php_version.clone().unwrap_or_else(|| "-".into()),
            match e.channel {
                CatalogChannelDto::Frozen => "frozen".into(),
                CatalogChannelDto::Live => "live".into(),
            },
            yes_no(e.installed).into(),
            notes.join(", "),
        ]);
    }
    table(&rows)
}

pub fn installed(entries: &[InstalledPackageDto]) -> String {
    if entries.is_empty() {
        return "No packages installed. Install one with `lampo package install php 8.3`.".into();
    }
    let mut rows = vec![row(["KIND", "VERSION", "PHP", "USED BY", "PATH"])];
    for e in entries {
        rows.push(vec![e.id.kind.clone(), e.id.version.clone(), e.id.php_version.clone().unwrap_or_else(|| "-".into()), e.used_by.join(","), e.path.clone()]);
    }
    table(&rows)
}

pub fn installed_one(entry: &InstalledPackageDto) -> String {
    let php = entry.id.php_version.as_ref().map(|p| format!(" for PHP {p}")).unwrap_or_default();
    format!("Installed {} {}{php} at {}", entry.id.kind, entry.id.version, entry.path)
}

pub fn databases(list: &DatabaseListDto) -> String {
    let mut out = format!("mysql {} at {}:{} (user {})\n", list.mysql_version, list.host, list.port, list.user);
    for name in &list.databases {
        let marker = if list.site_database.as_deref() == Some(name.as_str()) { "  (this site)" } else { "" };
        out.push_str(&format!("  {name}{marker}\n"));
    }
    if list.databases.is_empty() {
        out.push_str("  (no databases)\n");
    }
    out
}

pub fn doctor(report: &DoctorReportDto) -> String {
    let mut out = String::new();
    for c in &report.checks {
        let tag = match c.status {
            CheckStatus::Ok => "ok",
            CheckStatus::Warning => "warn",
            CheckStatus::Error => "error",
        };
        out.push_str(&format!("[{tag}] {}: {}\n", c.id, c.message));
        if let Some(fix) = &c.fix {
            out.push_str(&format!("       fix: {fix}\n"));
        }
    }
    out.push_str(if report.ok { "\nNo problems found." } else { "\nProblems found (exit code 14)." });
    out
}

pub fn shim_status(status: &ShimStatusDto) -> String {
    let mut out = format!("shim dir:  {}\ninstalled: {}\non PATH:   {}", status.shim_dir, yes_no(status.installed), yes_no(status.on_path));
    for s in &status.shadowed {
        out.push_str(&format!("\nshadowed:  {} by {} (the Lampo shim is {})", s.tool, s.shadowed_by, s.shim));
    }
    if !status.installed {
        out.push_str("\nRun `lampo shims install`.");
    }
    out
}

pub fn toolchain_env(env: &ToolchainEnvDto) -> String {
    let mut out = String::new();
    if let Some(site) = &env.site {
        out.push_str(&format!("site: {site}\n"));
    }
    out.push_str("PATH is prefixed with:\n");
    for d in &env.path_prepend {
        out.push_str(&format!("  {d}\n"));
    }
    out.push_str("variables:\n");
    for (k, v) in &env.vars {
        out.push_str(&format!("  {k}={v}\n"));
    }
    out
}

/// One-line description of what a shell fragment is for (used in `agent context`).
pub fn shell_name(shell: ShellKind) -> &'static str {
    match shell {
        ShellKind::Powershell => "powershell",
        ShellKind::Cmd => "cmd",
        ShellKind::Bash => "bash",
    }
}

/// Static facts about the CLI that every agent needs, appended to `agent context` (JSON under
/// `cli`, human as the closing section).
pub fn cli_guide() -> serde_json::Value {
    let exit_codes: Vec<serde_json::Value> = ExitCode::TABLE.iter().map(|(c, m)| serde_json::json!({ "code": c.code(), "meaning": m })).collect();
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "exitCodes": exit_codes,
        "jsonErrorShape": { "error": { "code": "kebab-case error code", "message": "what failed", "hint": "what to do next", "exitCode": 0, "details": "optional" } },
        "rules": [
            "Add --json to any command (except exec) for exactly one JSON document on stdout; diagnostics go to stderr; errors are JSON too.",
            "Nothing prompts. Destructive commands need an explicit flag (--yes, --drop-database).",
            "No UAC prompt unless --allow-elevation is given (exit code 7 tells you it is needed).",
            "Read-only commands work without the app; mutating ones start it headless unless --no-start (exit code 9).",
            "Site resolution order: --project, LAMPO_PROJECT, current folder (project or git worktree), global default PHP.",
            "php, composer, mysql, mysqldump and redis-cli on PATH are shims that already use this site's versions; no wrapper needed.",
            "Run commands in another toolchain with: lampo exec -- <command> [args...]."
        ],
    })
}

pub fn agent_context(ctx: &AgentContextDto) -> String {
    let mut out = String::from("# Lampo context\n\n");
    let r = &ctx.resolution;
    let kind = match r.kind {
        ResolvedKind::Project => "registered project",
        ResolvedKind::WorktreeEnvironment => "git worktree with its own environment",
        ResolvedKind::WorktreeWithoutEnvironment => "git worktree of a project, no environment yet (parent's data is shown)",
        ResolvedKind::GlobalDefault => "no project here: global default PHP only",
    };
    out.push_str(&format!("Resolved: {kind} (via {})\n", r.via));
    out.push_str(&format!("  app running: {}\n", yes_no(ctx.app_running)));
    if let Some(site) = &r.site {
        out.push_str(&format!("  site:        {site}\n"));
    }
    if let Some(name) = &ctx.project_name {
        out.push_str(&format!("  project:     {} ({name})\n", ctx.project_id.clone().unwrap_or_default()));
    }
    if let Some(root) = &ctx.root {
        out.push_str(&format!("  folder:      {root}\n"));
    }
    if let Some(wt) = &ctx.worktree {
        out.push_str(&format!(
            "  worktree:    {}{} (environment {})\n",
            wt.worktree_root,
            wt.branch.as_ref().map(|b| format!(" on branch {b}")).unwrap_or_default(),
            if wt.environment_up { "up" } else { "not created" }
        ));
    }
    out.push_str("\n## Web\n");
    match &ctx.url {
        Some(url) => {
            out.push_str(&format!("  url:       {url}\n"));
            if ctx.urls.len() > 1 {
                out.push_str(&format!("  all urls:  {}\n", ctx.urls.join(", ")));
            }
        }
        None => out.push_str("  url:       none (not served)\n"),
    }
    if let Some(doc) = &ctx.document_root {
        out.push_str(&format!("  docroot:   {doc}\n"));
    }
    out.push_str("\n## PHP\n");
    out.push_str(&format!("  version:   {}\n  binary:    {}\n", ctx.php.version, ctx.php.binary));
    if let Some(ini) = &ctx.php.ini_dir {
        out.push_str(&format!("  php.ini dir (PHPRC): {ini}\n"));
    }
    match &ctx.php.composer_phar {
        Some(phar) => out.push_str(&format!("  composer:  {phar} ({} channel)\n", ctx.php.composer_channel)),
        None => out.push_str("  composer:  not installed\n"),
    }
    out.push_str("  `php` and `composer` on PATH already use this version.\n");
    out.push_str("\n## Database\n");
    match &ctx.database {
        Some(db) => {
            out.push_str(&format!(
                "  host:      {}\n  port:      {}\n  user:      {}\n  password:  {:?}\n  database:  {}\n  mysql:     {}\n",
                db.host, db.port, db.user, db.password, db.database, db.mysql_version
            ));
            out.push_str("  Lampo never writes these into .env: set them in the project's own configuration.\n");
        }
        None => out.push_str("  none for this site\n"),
    }
    if !ctx.services.is_empty() {
        out.push_str("\n## Services\n");
        for s in &ctx.services {
            out.push_str(&format!("  {:<12} {}:{}\n", s.label, s.host, s.port));
        }
    }
    out.push_str("\n## Logs (read with `lampo logs <source> [--tail N] [--follow]`)\n");
    for l in &ctx.logs {
        out.push_str(&format!("  {:<22} {}{}\n", l.id, l.file, if l.exists { "" } else { "  (not created yet)" }));
    }
    out.push_str("\n## Commands\n");
    let width = ctx.commands.iter().map(|c| c.usage.chars().count()).max().unwrap_or(0);
    for c in &ctx.commands {
        let flags = match (c.mutating, c.requires_app) {
            (true, _) => " [changes state, uses the app]",
            (false, true) => " [uses the app]",
            (false, false) => "",
        };
        out.push_str(&format!("  {:<width$}  {}{flags}\n", c.usage, c.description));
    }
    if !ctx.hints.is_empty() {
        out.push_str("\n## Hints\n");
        for h in &ctx.hints {
            out.push_str(&format!("  - {h}\n"));
        }
    }
    out.push_str("\n## Conventions\n");
    if let Some(rules) = cli_guide()["rules"].as_array() {
        for rule in rules.iter().filter_map(|r| r.as_str()) {
            out.push_str(&format!("  - {rule}\n"));
        }
    }
    out.push_str("\n## Exit codes\n");
    for (code, meaning) in ExitCode::TABLE {
        out.push_str(&format!("  {:>2}  {meaning}\n", code.code()));
    }
    out.push_str("\n## JSON errors\n  {\"error\": {\"code\": \"not-found\", \"message\": \"...\", \"hint\": \"...\", \"exitCode\": 3}} on stdout, exit code as above.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_aligns_columns_and_trims_the_tail() {
        let text = table(&[row(["A", "BBBB", "C"]), row(["aaa", "b", ""])]);
        assert_eq!(text, "A    BBBB  C\naaa  b");
    }

    #[test]
    fn table_of_nothing_is_empty() {
        assert_eq!(table(&[]), "");
    }

    #[test]
    fn empty_sync_report_says_so() {
        assert_eq!(sync_report(&SyncReportDto::default()), "Nothing to do.");
    }
}
