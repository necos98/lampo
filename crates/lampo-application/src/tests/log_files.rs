//! Opening a log's file or folder, and exporting the lines the viewer shows.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lampo_domain::ports::{ExternalAppLauncher, LogExportWriter};
use lampo_domain::project::ProjectId;
use lampo_domain::{PortError, PortResult};

use crate::dto::LogExportRequest;
use crate::error::ErrorCode;
use crate::testing::*;
use crate::use_cases::log_files::MAX_EXPORT_LINES;
use crate::use_cases::*;

/// Records which launcher method opened which path; answers with `failure` when one is set.
#[derive(Default)]
struct RecordingLauncher {
    opened: Mutex<Vec<(&'static str, PathBuf)>>,
    failure: Option<PortError>,
}

impl RecordingLauncher {
    fn record(&self, how: &'static str, path: &Path) -> PortResult<()> {
        self.opened.lock().unwrap().push((how, path.to_path_buf()));
        self.failure.clone().map_or(Ok(()), Err)
    }

    fn opened(&self) -> Vec<(&'static str, PathBuf)> {
        self.opened.lock().unwrap().clone()
    }
}

impl ExternalAppLauncher for RecordingLauncher {
    fn open_folder(&self, folder: &Path) -> PortResult<()> {
        self.record("folder", folder)
    }
    fn open_file(&self, file: &Path) -> PortResult<()> {
        self.record("file", file)
    }
    fn open_in_vscode(&self, _folder: &Path) -> PortResult<()> {
        unreachable!("a log is never opened in VS Code")
    }
    fn open_terminal(&self, _project: &ProjectId, _folder: &Path) -> PortResult<()> {
        unreachable!("a log is never opened in a terminal")
    }
}

/// Records every export; answers with `failure` when one is set.
#[derive(Default)]
struct RecordingExportWriter {
    written: Mutex<Vec<(PathBuf, Vec<String>)>>,
    failure: Option<PortError>,
}

impl LogExportWriter for RecordingExportWriter {
    fn write_lines(&self, file: &Path, lines: &[String]) -> PortResult<()> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        self.written.lock().unwrap().push((file.to_path_buf(), lines.to_vec()));
        Ok(())
    }
}

fn open_log(w: &World, launcher: &Arc<RecordingLauncher>) -> OpenLog {
    let logs = Arc::new(LogQueries::new(Arc::new(FakeLogs::default()), w.projects.clone(), w.settings.clone(), w.services.clone(), w.paths.clone()));
    OpenLog::new(logs, w.fs.clone(), launcher.clone())
}

fn php_log(w: &World) -> PathBuf {
    w.paths.site_log_dir(&"shop".parse().unwrap()).join("php_errors.log")
}

#[test]
fn opens_the_file_of_a_source_and_its_folder() {
    let w = World::with_basics();
    let file = php_log(&w);
    let folder = file.parent().unwrap().to_path_buf();
    w.fs.add_file(&file);
    w.fs.add(&folder.display().to_string());
    let launcher = Arc::new(RecordingLauncher::default());
    let open = open_log(&w, &launcher);

    open.execute("php:shop", LogTarget::File).unwrap();
    open.execute(" php:shop ", LogTarget::Folder).unwrap();

    assert_eq!(launcher.opened(), vec![("file", file), ("folder", folder)]);
}

#[test]
fn a_log_not_written_yet_is_refused_before_launching() {
    let w = World::with_basics();
    let file = php_log(&w);
    let launcher = Arc::new(RecordingLauncher::default());
    let open = open_log(&w, &launcher);

    let err = open.execute("php:shop", LogTarget::File).unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert_eq!(err.message, format!("the log file does not exist yet: {}", file.display()));
    assert_eq!(err.details.unwrap()["path"], file.display().to_string(), "the GUI names the file");

    let folder = file.parent().unwrap();
    let err = open.execute("php:shop", LogTarget::Folder).unwrap_err();
    assert_eq!(err.message, format!("the log folder does not exist yet: {}", folder.display()));

    assert!(launcher.opened().is_empty());
}

#[test]
fn an_unknown_source_is_a_validation_error() {
    let w = World::with_basics();
    let launcher = Arc::new(RecordingLauncher::default());

    let err = open_log(&w, &launcher).execute("nginx", LogTarget::File).unwrap_err();

    assert_eq!(err.code, ErrorCode::Validation);
    assert!(launcher.opened().is_empty());
}

#[test]
fn a_failed_start_names_what_could_not_be_opened() {
    let w = World::with_basics();
    let file = php_log(&w);
    w.fs.add_file(&file);
    w.fs.add(&file.parent().unwrap().display().to_string());
    let launcher = Arc::new(RecordingLauncher { failure: Some(PortError::process("explorer.exe", "access denied")), ..Default::default() });
    let open = open_log(&w, &launcher);

    for (target, expected) in [(LogTarget::File, "could not open the log file: access denied"), (LogTarget::Folder, "could not open the log folder: access denied")] {
        let err = open.execute("php:shop", target).unwrap_err();
        assert_eq!(err.code, ErrorCode::ProcessFailed, "{target:?}");
        assert_eq!(err.message, expected);
    }
}

fn export(file: &str, lines: &[&str]) -> LogExportRequest {
    LogExportRequest { file: file.to_string(), lines: lines.iter().map(|l| l.to_string()).collect() }
}

#[test]
fn exports_the_lines_to_the_chosen_file() {
    let writer = Arc::new(RecordingExportWriter::default());

    ExportLogLines::new(writer.clone()).execute(&export(r"  C:\Users\dev\Desktop\shop.log ", &["a", "b"])).unwrap();

    assert_eq!(*writer.written.lock().unwrap(), vec![(PathBuf::from(r"C:\Users\dev\Desktop\shop.log"), vec!["a".to_string(), "b".to_string()])]);
}

#[test]
fn an_export_needs_an_absolute_file_and_a_bounded_number_of_lines() {
    let writer = Arc::new(RecordingExportWriter::default());
    let exporter = ExportLogLines::new(writer.clone());
    let too_many = LogExportRequest { file: r"C:\out.log".into(), lines: vec![String::new(); MAX_EXPORT_LINES + 1] };

    for (request, message) in [
        (export("  ", &["a"]), "an export file is required".to_string()),
        (export(r"..\out.log", &["a"]), r"the export file must be an absolute path: ..\out.log".to_string()),
        (too_many, format!("too many lines to export: {} (at most {MAX_EXPORT_LINES})", MAX_EXPORT_LINES + 1)),
    ] {
        let err = exporter.execute(&request).unwrap_err();
        assert_eq!(err.code, ErrorCode::Validation);
        assert_eq!(err.message, message);
    }
    assert!(writer.written.lock().unwrap().is_empty());
}

#[test]
fn a_file_that_cannot_be_written_is_an_io_error() {
    let writer = Arc::new(RecordingExportWriter { failure: Some(PortError::io(r"writing C:\out.log", "access denied")), ..Default::default() });

    let err = ExportLogLines::new(writer).execute(&export(r"C:\out.log", &["a"])).unwrap_err();

    assert_eq!(err.code, ErrorCode::Io);
}
