use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "LogSource"))]
#[serde(rename_all = "camelCase")]
pub struct LogSourceDto {
    /// `apache`, `apache-error`, `mysql-8.4`, `mysql-8.4-error`, `redis`,
    /// `access:<site>`, `error:<site>`, `php:<site>`.
    pub id: String,
    pub file: String,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LogTailRequest {
    pub source: String,
    pub lines: usize,
    pub since_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "LogLine"))]
#[serde(rename_all = "camelCase")]
pub struct LogLineDto {
    pub text: String,
    pub at_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "LogLines"))]
#[serde(rename_all = "camelCase")]
pub struct LogLinesDto {
    pub source: String,
    pub file: String,
    pub lines: Vec<LogLineDto>,
    /// Pass back to `read_from` to follow the file.
    pub next_offset: u64,
}

/// Lines the log viewer shows (after its filters), saved to `file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LogExportRequest {
    /// Absolute path of the file to write (replaced if it exists).
    pub file: String,
    pub lines: Vec<String>,
}
