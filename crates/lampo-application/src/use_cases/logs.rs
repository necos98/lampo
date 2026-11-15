use std::collections::BTreeSet;
use std::sync::Arc;

use lampo_domain::logs::{LogChunk, LogQuery, LogSource};
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{LogFileReader, ProjectRepository, SettingsRepository};
use lampo_domain::service::ServiceId;

use super::service_catalog::ServiceCatalog;
use super::support::parse_site;
use crate::dto::*;
use crate::error::{AppError, AppResult};

/// Log access (file-based, works without the app). `lampo logs --follow` polls `read_from`.
pub struct LogQueries {
    reader: Arc<dyn LogFileReader>,
    projects: Arc<dyn ProjectRepository>,
    settings: Arc<dyn SettingsRepository>,
    services: Arc<ServiceCatalog>,
    paths: LampoPaths,
}

impl LogQueries {
    pub fn new(reader: Arc<dyn LogFileReader>, projects: Arc<dyn ProjectRepository>, settings: Arc<dyn SettingsRepository>, services: Arc<ServiceCatalog>, paths: LampoPaths) -> Self {
        Self { reader, projects, settings, services, paths }
    }

    fn source_dto(&self, source: &LogSource) -> LogSourceDto {
        let file = source.file(&self.paths);
        // The reader yields an empty chunk at offset 0 for a missing file: a file that exists
        // but is empty is reported as missing too (nothing to read either way).
        let exists = self.reader.tail(&file, &LogQuery { max_lines: 0, since_unix_ms: None }).is_ok_and(|c| c.next_offset > 0);
        LogSourceDto { id: source.to_string(), file: file.display().to_string(), exists }
    }

    /// Sources of a site (`access/error/php` + the error logs of its services), or every
    /// service source when `None`: output and error log of each configured service and of
    /// every service a project uses.
    pub fn sources(&self, site: Option<&str>) -> AppResult<Vec<LogSourceDto>> {
        let sources = match site.map(str::trim).filter(|s| !s.is_empty()) {
            Some(raw) => {
                let site = parse_site(raw)?;
                let project = self.projects.get(site.project())?.ok_or_else(|| AppError::not_found("project", site.project()))?;
                LogSource::for_site(&site, self.services.with_error_log(project.environment.service_ids()))
            }
            None => {
                let mut ids: Vec<ServiceId> = self.services.configured(&self.settings.load()?);
                let used: BTreeSet<ServiceId> = self.projects.list()?.iter().flat_map(|p| p.environment.service_ids().collect::<Vec<_>>()).collect();
                ids.extend(used.into_iter().filter(|id| !ids.contains(id)).collect::<Vec<_>>());
                let mut out = Vec::new();
                for id in ids {
                    out.push(LogSource::ServiceOutput(id.clone()));
                    out.extend(self.services.with_error_log([id]).into_iter().map(LogSource::ServiceError));
                }
                out
            }
        };
        Ok(sources.iter().map(|s| self.source_dto(s)).collect())
    }

    /// Parses a source; service sources must name a registered kind (and an error log only
    /// for kinds that keep one).
    fn parse_source(&self, raw: &str) -> AppResult<LogSource> {
        let source: LogSource = raw.trim().parse()?;
        let known = match &source {
            LogSource::ServiceOutput(id) => self.services.descriptor(id.kind()).is_some(),
            LogSource::ServiceError(id) => self.services.descriptor(id.kind()).is_some_and(|d| d.has_error_log()),
            _ => true,
        };
        match known {
            true => Ok(source),
            false => Err(AppError::new(crate::error::ErrorCode::Validation, format!("unknown log source `{source}`"))),
        }
    }

    fn lines(source: &LogSource, file: &std::path::Path, chunk: LogChunk) -> LogLinesDto {
        LogLinesDto {
            source: source.to_string(),
            file: file.display().to_string(),
            lines: chunk.lines.into_iter().map(|l| LogLineDto { text: l.text, at_unix_ms: l.at_unix_ms }).collect(),
            next_offset: chunk.next_offset,
        }
    }

    pub fn tail(&self, request: &LogTailRequest) -> AppResult<LogLinesDto> {
        let source = self.parse_source(&request.source)?;
        let file = source.file(&self.paths);
        let chunk = self.reader.tail(&file, &LogQuery { max_lines: request.lines, since_unix_ms: request.since_unix_ms })?;
        Ok(Self::lines(&source, &file, chunk))
    }

    pub fn read_from(&self, source: &str, offset: u64) -> AppResult<LogLinesDto> {
        let source = self.parse_source(source)?;
        let file = source.file(&self.paths);
        let chunk = self.reader.read_from(&file, offset)?;
        Ok(Self::lines(&source, &file, chunk))
    }
}

/// Parses `--since` values: relative (`30s`, `10m`, `2h`, `1d`) or absolute Unix ms.
/// Returns the absolute instant in Unix ms. Pure (the caller passes `now_unix_ms`).
pub fn parse_since(value: &str, now_unix_ms: u64) -> Option<u64> {
    let v = value.trim();
    if let Ok(abs) = v.parse::<u64>() {
        return Some(abs);
    }
    let (num, unit) = v.split_at(v.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = num.parse().ok()?;
    let ms = match unit {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    Some(now_unix_ms.saturating_sub(n.saturating_mul(ms)))
}

#[cfg(test)]
mod tests {
    use super::parse_since;

    #[test]
    fn since_values() {
        assert_eq!(parse_since("10m", 1_000_000), Some(400_000));
        assert_eq!(parse_since("1700000000000", 0), Some(1_700_000_000_000));
        assert_eq!(parse_since("5x", 0), None);
        assert_eq!(parse_since("m", 0), None);
    }
}
