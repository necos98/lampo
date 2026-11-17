//! `status`, `up`/`down`/`restart`, `quit`.

use lampo_application::control::ControlRequest;
use lampo_application::dto::*;
use lampo_application::{AppResult, ErrorCode};

use super::Cmd;
use crate::cli::ServicesArgs;
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
