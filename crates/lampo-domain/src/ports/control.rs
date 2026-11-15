//! Local control channel between front-ends (the `lampo` CLI) and the running desktop app,
//! which is the single owner of service processes.
//!
//! Transport: a Windows named pipe restricted to the current user, carrying newline-delimited
//! UTF-8 JSON frames. This layer moves opaque frames only; the request/response/event schema
//! and protocol version live in `lampo-application::control`. Implemented by
//! `lampo-infra-services` (`control_pipe` module).
//!
//! Exchange: the client sends one request frame; the server sends zero or more event frames
//! (progress, log lines) and then exactly one final response frame, then closes.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::error::PortResult;
use crate::progress::CancelFlag;

/// Receives intermediate frames of one exchange (client side) or sends them (server side).
pub trait ControlFrameSink: Send + Sync {
    fn frame(&self, json: String);
}

/// Handles one request on the server side. Implemented by the application's dispatcher.
#[async_trait]
pub trait ControlRequestHandler: Send + Sync {
    /// Returns the final response frame; may push intermediate frames to `events`.
    /// The server adapter sets `cancel` when the client disconnects (e.g. Ctrl-C in the CLI).
    async fn handle(&self, request_json: String, events: &dyn ControlFrameSink, cancel: &CancelFlag) -> String;
}

/// Client end of the channel.
#[async_trait]
pub trait ControlChannelClient: Send + Sync {
    /// Cheap check that an app instance is listening (pipe exists). Never starts anything.
    fn is_server_running(&self) -> bool;
    /// Sends `request_json`, forwards intermediate frames to `events`, returns the final frame.
    /// `PortError::NotFound { entity: "app", .. }` when no server is listening.
    async fn exchange(&self, request_json: String, events: &dyn ControlFrameSink, timeout: Duration) -> PortResult<String>;
}

/// Server end of the channel, hosted by the desktop app.
#[async_trait]
pub trait ControlChannelServer: Send + Sync {
    /// Accepts connections until the process exits, serving each with `handler` concurrently.
    async fn serve(&self, handler: Arc<dyn ControlRequestHandler>) -> PortResult<()>;
}

/// Starts the desktop app headless (tray only, no webview window) when it is not running.
pub trait AppLauncher: Send + Sync {
    /// Spawns the app detached with `--headless` and returns immediately; the caller then polls
    /// [`ControlChannelClient::is_server_running`]. `NotFound` if the app executable is missing.
    fn launch_headless(&self) -> PortResult<()>;
}
