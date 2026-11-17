//! The outcome of one environment sync: the report, and the failures that did not prevent the
//! other steps.

use serde_json::{Map, Value};

use crate::dto::SyncReportDto;
use crate::error::{AppError, AppResult, ErrorCode};

/// Key of the sync report in the details of a failed sync.
const REPORT: &str = "report";
/// Key of the list of every failure in the details of a failed sync.
const ERRORS: &str = "errors";
/// Keys of one entry of [`ERRORS`].
const CODE: &str = "code";
const MESSAGE: &str = "message";
const DETAILS: &str = "details";

/// Collects the outcome of one sync. Every failure is kept: the first one is the error returned
/// at the end (after everything else was applied), the later ones are also listed in the
/// report's warnings, and all of them are in `details.errors` (`[{code, message, details?}]`),
/// so a front-end sees e.g. an `ElevationRequired` that came after a `PortInUse`.
#[derive(Default)]
pub(crate) struct Outcome {
    pub report: SyncReportDto,
    errors: Vec<AppError>,
}

impl Outcome {
    pub fn fail(&mut self, error: AppError) {
        if !self.errors.is_empty() {
            self.report.warnings.push(error.message.clone());
        }
        self.errors.push(error);
    }

    /// The report, or the first failure with `details.report` and `details.errors` added to its
    /// own details.
    pub fn finish(self) -> AppResult<SyncReportDto> {
        let mut errors = self.errors.into_iter();
        let Some(mut first) = errors.next() else { return Ok(self.report) };
        let summary: Vec<Value> = std::iter::once(&first).chain(errors.as_slice()).map(error_summary).collect();
        let mut details = match first.details.take() {
            Some(Value::Object(map)) => map,
            _ => Map::new(),
        };
        details.insert(REPORT.into(), serde_json::to_value(&self.report).unwrap_or_default());
        details.insert(ERRORS.into(), Value::Array(summary));
        Err(first.with_details(Value::Object(details)))
    }
}

fn error_summary(error: &AppError) -> Value {
    let mut summary = serde_json::json!({ CODE: error.code, MESSAGE: error.message });
    if let Some(details) = &error.details {
        summary[DETAILS] = details.clone();
    }
    summary
}

/// The failure `error` of a sync stands for when the failures coded `ignored` do not count:
/// `error` itself unless it is one of them, otherwise the first other failure listed in its
/// `details.errors`, with its own details plus the same `report` and `errors`. `None` when every
/// failure was `ignored`.
pub(crate) fn first_failure_other_than(error: AppError, ignored: ErrorCode) -> Option<AppError> {
    if error.code != ignored {
        return Some(error);
    }
    let details = error.details?;
    let listed = details.get(ERRORS)?.as_array()?;
    let (code, entry) = listed.iter().filter_map(|entry| Some((listed_code(entry)?, entry))).find(|(code, _)| *code != ignored)?;
    let message = entry.get(MESSAGE).and_then(Value::as_str).unwrap_or_default();
    let mut own = match entry.get(DETAILS) {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    for key in [REPORT, ERRORS] {
        if let Some(value) = details.get(key) {
            own.insert(key.into(), value.clone());
        }
    }
    Some(AppError::new(code, message).with_details(Value::Object(own)))
}

/// The code of one entry of `details.errors`.
fn listed_code(entry: &Value) -> Option<ErrorCode> {
    serde_json::from_value(entry.get(CODE)?.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_failure_is_the_report() {
        let mut out = Outcome::default();
        out.report.started.push("redis".into());
        assert_eq!(out.finish().unwrap().started, ["redis"]);
    }

    #[test]
    fn every_failure_keeps_its_code_and_the_first_one_is_returned() {
        let mut out = Outcome::default();
        out.fail(AppError::new(ErrorCode::PortInUse, "port 80 is in use").with_details(serde_json::json!({ "port": 80 })));
        out.fail(AppError::new(ErrorCode::ElevationRequired, "the hosts file must be updated"));
        let err = out.finish().unwrap_err();
        assert_eq!(err.code, ErrorCode::PortInUse);
        let details = err.details.unwrap();
        assert_eq!(details["port"], 80, "the first error's own details are kept");
        assert_eq!(
            details["errors"],
            serde_json::json!([
                { "code": "port-in-use", "message": "port 80 is in use", "details": { "port": 80 } },
                { "code": "elevation-required", "message": "the hosts file must be updated" },
            ])
        );
        assert_eq!(details["report"]["warnings"], serde_json::json!(["the hosts file must be updated"]));
    }

    #[test]
    fn the_first_failure_of_another_code_comes_with_its_own_details() {
        let mut out = Outcome::default();
        out.fail(AppError::new(ErrorCode::ElevationRequired, "the hosts file must be updated"));
        out.fail(AppError::new(ErrorCode::PortInUse, "port 80 is in use").with_details(serde_json::json!({ "port": 80 })));
        out.fail(AppError::new(ErrorCode::ProcessFailed, "mysqld exited during startup"));
        let err = first_failure_other_than(out.finish().unwrap_err(), ErrorCode::ElevationRequired).unwrap();
        assert_eq!(err.code, ErrorCode::PortInUse);
        assert_eq!(err.message, "port 80 is in use");
        let details = err.details.unwrap();
        assert_eq!(details["port"], 80);
        assert_eq!(details["errors"].as_array().unwrap().len(), 3, "every failure is still listed");
        assert_eq!(details["report"]["warnings"], serde_json::json!(["port 80 is in use", "mysqld exited during startup"]));
    }

    #[test]
    fn only_ignored_failures_are_none_and_another_first_failure_is_itself() {
        let mut out = Outcome::default();
        out.fail(AppError::new(ErrorCode::ElevationRequired, "the hosts file must be updated"));
        assert_eq!(first_failure_other_than(out.finish().unwrap_err(), ErrorCode::ElevationRequired), None);

        let busy = AppError::new(ErrorCode::PortInUse, "port 80 is in use");
        assert_eq!(first_failure_other_than(busy.clone(), ErrorCode::ElevationRequired), Some(busy));
    }
}
