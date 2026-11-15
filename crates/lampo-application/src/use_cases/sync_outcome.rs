//! The outcome of one environment sync: the report, and the failures that did not prevent the
//! other steps.

use crate::dto::SyncReportDto;
use crate::error::{AppError, AppResult};

/// Collects the outcome of one sync. Every failure is kept: the first one is the error returned
/// at the end (after everything else was applied), the later ones are also listed in the
/// report's warnings, and all of them are in `details.errors` (`[{code, message}]`), so a front-end
/// sees e.g. an `ElevationRequired` that came after a `PortInUse`.
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
        let summary: Vec<serde_json::Value> = std::iter::once(&first).chain(errors.as_slice()).map(error_summary).collect();
        let mut details = match first.details.take() {
            Some(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };
        details.insert("report".into(), serde_json::to_value(&self.report).unwrap_or_default());
        details.insert("errors".into(), serde_json::Value::Array(summary));
        Err(first.with_details(serde_json::Value::Object(details)))
    }
}

fn error_summary(error: &AppError) -> serde_json::Value {
    serde_json::json!({ "code": error.code, "message": error.message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

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
                { "code": "port-in-use", "message": "port 80 is in use" },
                { "code": "elevation-required", "message": "the hosts file must be updated" },
            ])
        );
        assert_eq!(details["report"]["warnings"], serde_json::json!(["the hosts file must be updated"]));
    }
}
