//! Acting on a log from the desktop: opening its file or folder in the user's apps, and saving
//! the lines the viewer shows to a file of the user's choice.

use std::path::Path;
use std::sync::Arc;

use lampo_domain::PortError;
use lampo_domain::ports::{ExternalAppLauncher, FileSystemInspector, LogExportWriter};

use super::logs::LogQueries;
use super::support::validation;
use crate::dto::LogExportRequest;
use crate::error::{AppError, AppResult, ErrorCode};

/// Most lines one export may carry; far above what the viewer keeps (a few thousand).
pub const MAX_EXPORT_LINES: usize = 100_000;

/// What of a log to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogTarget {
    /// The log file, in the app associated with it.
    File,
    /// The folder holding the log file, in the file manager.
    Folder,
}

impl LogTarget {
    /// What is opened, as in "the log {noun} does not exist yet".
    fn noun(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
        }
    }
}

/// Opens a log source's file or folder in the user's apps. Starts a process of the user and
/// changes nothing in Lampo.
pub struct OpenLog {
    logs: Arc<LogQueries>,
    fs: Arc<dyn FileSystemInspector>,
    launcher: Arc<dyn ExternalAppLauncher>,
}

impl OpenLog {
    pub fn new(logs: Arc<LogQueries>, fs: Arc<dyn FileSystemInspector>, launcher: Arc<dyn ExternalAppLauncher>) -> Self {
        Self { logs, fs, launcher }
    }

    /// `Validation` for an unknown source, or when the file (or its folder) has not been
    /// written yet (`details.path`); `ProcessFailed` when the app cannot be started.
    pub fn execute(&self, source: &str, target: LogTarget) -> AppResult<()> {
        let file = self.logs.file_of(source)?;
        let launched = match target {
            LogTarget::File => {
                self.ensure_exists(&file, target, self.fs.is_file(&file))?;
                self.launcher.open_file(&file)
            }
            LogTarget::Folder => {
                let folder = file.parent().unwrap_or(&file);
                self.ensure_exists(folder, target, self.fs.is_dir(folder))?;
                self.launcher.open_folder(folder)
            }
        };
        launched.map_err(|e| launch_error(target, e))
    }

    fn ensure_exists(&self, path: &Path, target: LogTarget, exists: bool) -> AppResult<()> {
        if exists {
            return Ok(());
        }
        let shown = path.display().to_string();
        Err(AppError::new(ErrorCode::Validation, format!("the log {} does not exist yet: {shown}", target.noun())).with_details(serde_json::json!({ "path": shown })))
    }
}

/// The error a front-end shows when the app for `target` could not be started.
fn launch_error(target: LogTarget, error: PortError) -> AppError {
    match error {
        PortError::Process { message, .. } => AppError::new(ErrorCode::ProcessFailed, format!("could not open the log {}: {message}", target.noun())),
        other => other.into(),
    }
}

/// Saves the lines the log viewer shows to a file the user picked.
pub struct ExportLogLines {
    writer: Arc<dyn LogExportWriter>,
}

impl ExportLogLines {
    pub fn new(writer: Arc<dyn LogExportWriter>) -> Self {
        Self { writer }
    }

    /// `Validation` for a missing or relative path or more than [`MAX_EXPORT_LINES`] lines;
    /// `Io` when the file cannot be written.
    pub fn execute(&self, request: &LogExportRequest) -> AppResult<()> {
        let file = Path::new(request.file.trim());
        if file.as_os_str().is_empty() {
            return Err(validation("an export file is required"));
        }
        if !file.is_absolute() {
            return Err(validation(format!("the export file must be an absolute path: {}", file.display())));
        }
        if request.lines.len() > MAX_EXPORT_LINES {
            return Err(validation(format!("too many lines to export: {} (at most {MAX_EXPORT_LINES})", request.lines.len())));
        }
        Ok(self.writer.write_lines(file, &request.lines)?)
    }
}
