//! `logs`: list the sources, print (and follow) a log filtered by time and level, or open its
//! file or folder in the user's apps.

use lampo_application::AppResult;
use lampo_application::dto::*;
use lampo_application::use_cases::LogTarget;
use lampo_application::use_cases::logs::parse_since;
use lampo_domain::log_level::LevelFilter;
use serde::Serialize;

use super::{Cmd, validation};
use crate::cli::LogsArgs;
use crate::render;

/// `--json` result of `logs --open` / `--open-folder`.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LogOpenedDto {
    pub source: String,
    /// `file` or `folder`.
    pub target: &'static str,
}

/// `access`, `error` and `php` mean "of the resolved site".
fn expand_source(c: &Cmd<'_>, source: &str) -> AppResult<String> {
    match source {
        "access" | "error" | "php" => Ok(format!("{source}:{}", c.site_id()?)),
        other => Ok(other.to_string()),
    }
}

pub async fn logs(c: &Cmd<'_>, args: LogsArgs) -> AppResult<i32> {
    let Some(source) = &args.source else {
        // Without a source: what can be read (the resolved site's sources if there is one).
        let site = c.resolved_site().ok().and_then(|r| r.site);
        let sources = c.ctx.logs.sources(site.as_deref())?;
        c.io.emit(&sources, |s| render::log_sources(s));
        return Ok(0);
    };
    let source = expand_source(c, source)?;
    if let Some(target) = open_target(&args) {
        return open(c, source, target);
    }
    let level = LevelFilter::from(args.level);
    let since_unix_ms = match &args.since {
        Some(value) => Some(parse_since(value, c.ctx.env.now_unix_ms).ok_or_else(|| validation(format!("invalid --since `{value}`: use 30s, 10m, 2h, 1d or a Unix timestamp in ms")))?),
        None => None,
    };
    let mut first = c.ctx.logs.tail(&LogTailRequest { source: source.clone(), lines: args.tail, since_unix_ms })?;
    keep_level(&mut first.lines, level);
    if !args.follow {
        c.io.emit(&first, render::log_lines);
        return Ok(0);
    }

    // Follow: NDJSON (one LogLineDto per line) or plain lines, until Ctrl+C or a closed pipe.
    let print = |lines: &[LogLineDto]| -> bool { lines.iter().all(|l| if c.io.json { c.io.emit_ndjson(l) } else { c.io.out_line(l.text.trim_end_matches(['\r', '\n'])) }) };
    if !print(&first.lines) {
        return Ok(0);
    }
    let mut offset = first.next_offset;
    loop {
        tokio::select! {
            _ = c.ctx.interrupt.wait() => return Ok(0),
            _ = tokio::time::sleep(c.ctx.env.follow_interval) => {}
        }
        let mut more = c.ctx.logs.read_from(&source, offset)?;
        offset = more.next_offset;
        keep_level(&mut more.lines, level);
        if !print(&more.lines) {
            return Ok(0);
        }
    }
}

/// Drops the lines below `level`, as the app's viewer does.
fn keep_level(lines: &mut Vec<LogLineDto>, level: LevelFilter) {
    lines.retain(|line| level.keeps(&line.text));
}

fn open_target(args: &LogsArgs) -> Option<LogTarget> {
    match (args.open, args.open_folder) {
        (true, _) => Some(LogTarget::File),
        (_, true) => Some(LogTarget::Folder),
        _ => None,
    }
}

/// Starts the user's app on the log from this process (the Lampo app need not run).
fn open(c: &Cmd<'_>, source: String, target: LogTarget) -> AppResult<i32> {
    c.ctx.log_opener.open(&source, target)?;
    let opened = LogOpenedDto {
        source,
        target: match target {
            LogTarget::File => "file",
            LogTarget::Folder => "folder",
        },
    };
    c.io.emit(&opened, |o| format!("opened the log {} of `{}`", o.target, o.source));
    Ok(0)
}
