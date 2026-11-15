//! `status`, `up`/`down`/`restart`, `logs`.

use lampo_application::control::ControlRequest;
use lampo_application::dto::*;
use lampo_application::use_cases::logs::parse_since;
use lampo_application::{AppResult, ErrorCode};

use super::{Cmd, validation};
use crate::cli::{LogsArgs, ServicesArgs};
use crate::render;

pub enum Action {
    Up,
    Down,
    Restart,
}

/// App running: its live view. Otherwise (or if it vanished meanwhile) the offline view.
pub async fn status(c: &Cmd<'_>) -> AppResult<i32> {
    let live = if c.ctx.app.is_running() {
        match c.app_if_running::<StatusDto>(ControlRequest::Status).await {
            Ok(status) => Some(status),
            Err(e) if e.code == ErrorCode::AppUnavailable => None,
            Err(e) => return Err(e),
        }
    } else {
        None
    };
    let status = match live {
        Some(status) => status,
        None => c.ctx.offline_status.status().await?,
    };
    c.io.emit(&status, render::status);
    Ok(0)
}

pub async fn control(c: &Cmd<'_>, action: Action, args: ServicesArgs) -> AppResult<i32> {
    let request = ServicesRequest { services: args.services };
    let request = match action {
        Action::Up => ControlRequest::ServicesUp(request),
        Action::Down => ControlRequest::ServicesDown(request),
        Action::Restart => ControlRequest::ServicesRestart(request),
    };
    let report: SyncReportDto = c.app(request).await?;
    c.io.emit(&report, render::sync_report);
    Ok(0)
}

/// How long `lampo quit` waits for the app to be gone (services stop gracefully first).
const QUIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Asks a running app to stop its services and exit, and waits until it has; nothing to do when
/// it is not running. `Timeout` if it is still there after [`QUIT_TIMEOUT`].
pub async fn quit(c: &Cmd<'_>) -> AppResult<i32> {
    let was_running = c.ctx.app.is_running();
    if was_running {
        match c.app_if_running::<serde_json::Value>(ControlRequest::Quit).await {
            Ok(_) => {}
            Err(e) if e.code == ErrorCode::AppUnavailable => {}
            Err(e) => return Err(e),
        }
        let deadline = tokio::time::Instant::now() + QUIT_TIMEOUT;
        while c.ctx.app.is_running() {
            if tokio::time::Instant::now() >= deadline {
                return Err(lampo_application::AppError::new(ErrorCode::Timeout, "the Lampo app did not exit in time"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
    let result = serde_json::json!({ "wasRunning": was_running });
    c.io.emit(&result, |_| if was_running { "Lampo stopped its services and exited.".to_string() } else { "Lampo is not running.".to_string() });
    Ok(0)
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
    let since_unix_ms = match &args.since {
        Some(value) => Some(parse_since(value, c.ctx.env.now_unix_ms).ok_or_else(|| validation(format!("invalid --since `{value}`: use 30s, 10m, 2h, 1d or a Unix timestamp in ms")))?),
        None => None,
    };
    let first = c.ctx.logs.tail(&LogTailRequest { source: source.clone(), lines: args.tail, since_unix_ms })?;
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
        let more = c.ctx.logs.read_from(&source, offset)?;
        offset = more.next_offset;
        if !print(&more.lines) {
            return Ok(0);
        }
    }
}
