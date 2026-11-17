//! How failures are reported: the exit code (see `exit_code.rs`), two lines on stderr, and in
//! `--json` mode one JSON document on stdout with a stable shape:
//!
//! ```json
//! { "error": { "code": "not-found", "message": "...", "hint": "...", "exitCode": 3, "details": {} } }
//! ```
//!
//! `code` is the stable [`ErrorCode`] string (or `usage` for a bad command line), `hint` is an
//! actionable English sentence (never absent), `details` appears only when the app supplied some.

use lampo_application::{AppError, ErrorCode};
use serde_json::{Value, json};

use crate::exit_code::ExitCode;
use crate::io::Io;

/// Error code string for command-line mistakes (not an application error).
pub const USAGE_CODE: &str = "usage";

/// An actionable next step for an error code.
pub fn hint_for(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::Validation => "Check the arguments (`lampo <command> --help` shows the accepted values) and retry.",
        ErrorCode::NotFound => "Check the name with `lampo project list`, `lampo worktree list`, `lampo package list` or `lampo logs`.",
        ErrorCode::Conflict => "The target already exists or is in use: pick another id/port/name, or inspect it with `lampo project list` and `lampo status`.",
        ErrorCode::ProjectNotResolved => "Run inside a registered project folder, pass --project <id>, set LAMPO_PROJECT, or register the folder with `lampo project create`.",
        ErrorCode::ElevationRequired => "Re-run with --allow-elevation to accept the UAC prompt for the hosts file, or use localhost addressing (the default of `lampo project create`).",
        ErrorCode::ElevationDenied => "The UAC prompt was refused. Retry with --allow-elevation and accept it, or avoid hosts-file names.",
        ErrorCode::AppUnavailable => "Start the Lampo desktop app (or drop --no-start so the CLI starts it headless), then retry. `lampo doctor` shows what is wrong.",
        ErrorCode::PackageMissing => "Install the missing package with `lampo package install <kind> <version>` (see `lampo package list`).",
        ErrorCode::Network => "Check the internet connection and retry.",
        ErrorCode::ChecksumMismatch => "The download is corrupt or was tampered with: retry, and report it if it persists.",
        ErrorCode::ConfigRejected => "Apache rejected the generated configuration: see `details.output` and `lampo logs apache-error`.",
        ErrorCode::PortInUse => {
            "Another program uses the port: stop it, or (with the user's consent) move Lampo's service with `lampo settings set --apache-port|--mysql-port|--redis-port`; `lampo doctor` lists conflicts."
        }
        ErrorCode::ProcessFailed => "An external program failed: see the message and `lampo logs`; `lampo doctor` may explain why.",
        ErrorCode::Io => "A file operation failed: check the path and permissions.",
        ErrorCode::Cancelled => "The operation was cancelled; run it again to retry.",
        ErrorCode::Timeout => "The operation timed out; run it again, and check `lampo status`.",
        ErrorCode::Unsupported => "This operation is not supported yet.",
        ErrorCode::ProtocolMismatch => "The CLI and the app have different versions: update both to the same release and restart the app.",
        ErrorCode::Internal => "Unexpected failure: re-run with --json and report the output; `lampo doctor` may help.",
    }
}

/// The hint for `error`: its code's, plus the `--allow-elevation` one when another failure of
/// the same operation needs elevation (`details.errors` of a sync, e.g. an `ElevationRequired`
/// that came after a `PortInUse`).
pub fn hint(error: &AppError) -> String {
    let own = hint_for(error.code);
    if error.code != ErrorCode::ElevationRequired && also_failed_with(error, ErrorCode::ElevationRequired) {
        return format!("{own} Also: {}", hint_for(ErrorCode::ElevationRequired));
    }
    own.to_string()
}

/// `true` when `details.errors` (`[{code, message, details?}]`) lists `code`.
fn also_failed_with(error: &AppError, code: ErrorCode) -> bool {
    let Ok(wanted) = serde_json::to_value(code) else { return false };
    let errors = error.details.as_ref().and_then(|d| d.get("errors")).and_then(Value::as_array);
    errors.is_some_and(|errors| errors.iter().any(|e| e.get("code") == Some(&wanted)))
}

/// The `error` object for an application error.
pub fn error_object(error: &AppError) -> Value {
    let code = serde_json::to_value(error.code).unwrap_or_else(|_| json!("internal"));
    let mut object = json!({
        "code": code,
        "message": error.message,
        "hint": hint(error),
        "exitCode": ExitCode::from(error.code).code(),
    });
    if let Some(details) = &error.details {
        object["details"] = details.clone();
    }
    object
}

/// The `error` object for a command-line mistake.
pub fn usage_error_object(message: &str) -> Value {
    json!({
        "code": USAGE_CODE,
        "message": message,
        "hint": "Run `lampo --help` or `lampo <command> --help` for the accepted syntax.",
        "exitCode": ExitCode::Usage.code(),
    })
}

/// Reports an application error and returns the process exit code.
pub fn report(io: &Io, error: &AppError) -> i32 {
    if io.json {
        io.out_line(&json!({ "error": error_object(error) }).to_string());
    }
    io.err_line(&format!("lampo: {}", error.message));
    io.err_line(&format!("hint: {}", hint(error)));
    ExitCode::from(error.code).code()
}

/// Reports a command-line mistake (clap's message) and returns exit code 2.
pub fn report_usage(io: &Io, message: &str) -> i32 {
    if io.json {
        io.out_line(&json!({ "error": usage_error_object(message.trim()) }).to_string());
    }
    io.err_line(message.trim_end());
    ExitCode::Usage.code()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::captured;

    /// Exhaustive on purpose: adding an `ErrorCode` breaks the build here until it is mapped.
    fn all_codes() -> Vec<(ErrorCode, ExitCode)> {
        use ErrorCode as E;
        let probe = |c: ErrorCode| match c {
            E::Validation
            | E::NotFound
            | E::Conflict
            | E::ProjectNotResolved
            | E::ElevationRequired
            | E::ElevationDenied
            | E::AppUnavailable
            | E::PackageMissing
            | E::Network
            | E::ChecksumMismatch
            | E::ConfigRejected
            | E::PortInUse
            | E::ProcessFailed
            | E::Io
            | E::Cancelled
            | E::Timeout
            | E::Unsupported
            | E::ProtocolMismatch
            | E::Internal => c,
        };
        let expected = [
            (E::Validation, ExitCode::Validation),
            (E::NotFound, ExitCode::NotFound),
            (E::Conflict, ExitCode::Conflict),
            (E::ProjectNotResolved, ExitCode::ProjectNotResolved),
            (E::ElevationRequired, ExitCode::ElevationRequired),
            (E::ElevationDenied, ExitCode::ElevationDenied),
            (E::AppUnavailable, ExitCode::AppUnavailable),
            (E::PackageMissing, ExitCode::PackageMissing),
            (E::Network, ExitCode::Download),
            (E::ChecksumMismatch, ExitCode::Download),
            (E::ConfigRejected, ExitCode::External),
            (E::PortInUse, ExitCode::Conflict),
            (E::ProcessFailed, ExitCode::External),
            (E::Io, ExitCode::Failure),
            (E::Cancelled, ExitCode::Interrupted),
            (E::Timeout, ExitCode::Interrupted),
            (E::Unsupported, ExitCode::Failure),
            (E::ProtocolMismatch, ExitCode::ProtocolMismatch),
            (E::Internal, ExitCode::Failure),
        ];
        expected.into_iter().map(|(c, e)| (probe(c), e)).collect()
    }

    #[test]
    fn every_error_kind_maps_to_its_documented_exit_code() {
        for (code, exit) in all_codes() {
            assert_eq!(ExitCode::from(code), exit, "{code:?}");
        }
    }

    #[test]
    fn every_error_kind_has_a_hint_and_json_shape() {
        for (code, exit) in all_codes() {
            let error = AppError::new(code, "boom");
            let object = error_object(&error);
            assert!(!object["hint"].as_str().unwrap().is_empty(), "{code:?}");
            assert_eq!(object["exitCode"], exit.code());
            assert_eq!(object["message"], "boom");
            assert!(object["code"].is_string());
            assert!(object.get("details").is_none());
        }
    }

    #[test]
    fn json_error_is_one_document_on_stdout() {
        let (io, out, err) = captured(true);
        let error = AppError::new(ErrorCode::NotFound, "project `x` not found").with_details(json!({"id": "x"}));
        assert_eq!(report(&io, &error), 3);
        let doc: Value = serde_json::from_str(out.text().trim()).unwrap();
        assert_eq!(out.text().lines().count(), 1);
        assert_eq!(doc["error"]["code"], "not-found");
        assert_eq!(doc["error"]["exitCode"], 3);
        assert_eq!(doc["error"]["details"]["id"], "x");
        assert!(err.text().starts_with("lampo: project `x` not found"));
    }

    #[test]
    fn human_error_goes_to_stderr_only() {
        let (io, out, err) = captured(false);
        assert_eq!(report(&io, &AppError::new(ErrorCode::ElevationRequired, "hosts needs UAC")), 7);
        assert_eq!(out.text(), "");
        assert!(err.text().contains("--allow-elevation"));
    }

    #[test]
    fn a_sync_that_also_needs_elevation_suggests_allow_elevation() {
        let errors = json!({ "errors": [
            { "code": "port-in-use", "message": "port 80 is in use" },
            { "code": "elevation-required", "message": "the hosts file must be updated" },
        ] });
        let (io, out, err) = captured(true);
        assert_eq!(report(&io, &AppError::new(ErrorCode::PortInUse, "port 80 is in use").with_details(errors)), 5, "the exit code is the first failure's");
        let doc: Value = serde_json::from_str(out.text().trim()).unwrap();
        let shown = doc["error"]["hint"].as_str().unwrap();
        assert!(shown.starts_with(hint_for(ErrorCode::PortInUse)) && shown.contains("--allow-elevation"), "{shown}");
        assert!(err.text().contains("--allow-elevation"));

        let alone = AppError::new(ErrorCode::PortInUse, "port 80 is in use").with_details(json!({ "errors": [{ "code": "port-in-use", "message": "x" }] }));
        assert_eq!(hint(&alone), hint_for(ErrorCode::PortInUse));
        assert_eq!(hint(&AppError::new(ErrorCode::ElevationRequired, "x")), hint_for(ErrorCode::ElevationRequired), "not suggested twice");
    }

    #[test]
    fn usage_errors_are_json_too() {
        let (io, out, _) = captured(true);
        assert_eq!(report_usage(&io, "error: unknown flag"), 2);
        let doc: Value = serde_json::from_str(out.text().trim()).unwrap();
        assert_eq!(doc["error"]["code"], "usage");
        assert_eq!(doc["error"]["exitCode"], 2);
    }
}
