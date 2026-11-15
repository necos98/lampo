//! Log sources and how they map to files. Logs are plain files under `%LOCALAPPDATA%\Lampo\logs`,
//! so they can be read (tail / since / follow) without the app running.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::paths::LampoPaths;
use crate::service::ServiceId;
use crate::site::SiteId;

/// One readable log. Text form (CLI): `apache`, `apache-error`, `mysql-8.4`, `mysql-8.4-error`,
/// `redis`, `access:<site>`, `error:<site>`, `php:<site>` where `<site>` is `shop` or
/// `shop@feature-x`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum LogSource {
    /// Captured stdout/stderr of a supervised service.
    ServiceOutput(ServiceId),
    /// A service's own error log (Apache's global error log, MySQL's `log-error`), for kinds
    /// whose descriptor `has_error_log`.
    ServiceError(ServiceId),
    /// Apache access log of a site's vhost.
    SiteAccess(SiteId),
    /// Apache error log of a site's vhost.
    SiteError(SiteId),
    /// PHP `error_log` of a site (set in its generated php.ini).
    SitePhp(SiteId),
}

impl LogSource {
    /// All sources relevant to a site: vhost access/error, PHP errors, then the error logs of
    /// `services` (the site's services that keep one).
    pub fn for_site(site: &SiteId, services: impl IntoIterator<Item = ServiceId>) -> Vec<LogSource> {
        let mut v = vec![Self::SiteAccess(site.clone()), Self::SiteError(site.clone()), Self::SitePhp(site.clone())];
        v.extend(services.into_iter().map(Self::ServiceError));
        v
    }

    pub fn file(&self, paths: &LampoPaths) -> PathBuf {
        match self {
            Self::ServiceOutput(s) => paths.service_log_dir(s).join("output.log"),
            Self::ServiceError(s) => paths.service_log_dir(s).join("error.log"),
            Self::SiteAccess(s) => paths.site_log_dir(s).join("access.log"),
            Self::SiteError(s) => paths.site_log_dir(s).join("error.log"),
            Self::SitePhp(s) => paths.site_log_dir(s).join("php_errors.log"),
        }
    }
}

impl fmt::Display for LogSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServiceOutput(s) => write!(f, "{s}"),
            Self::ServiceError(s) => write!(f, "{s}-error"),
            Self::SiteAccess(p) => write!(f, "access:{p}"),
            Self::SiteError(p) => write!(f, "error:{p}"),
            Self::SitePhp(p) => write!(f, "php:{p}"),
        }
    }
}

impl FromStr for LogSource {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || DomainError::InvalidLogSource(s.to_string());
        if let Some((kind, site)) = s.split_once(':') {
            let site: SiteId = site.parse().map_err(|_| bad())?;
            return match kind {
                "access" => Ok(Self::SiteAccess(site)),
                "error" => Ok(Self::SiteError(site)),
                "php" => Ok(Self::SitePhp(site)),
                _ => Err(bad()),
            };
        }
        if let Some(service) = s.strip_suffix("-error") {
            return service.parse::<ServiceId>().map(Self::ServiceError).map_err(|_| bad());
        }
        s.parse::<ServiceId>().map(Self::ServiceOutput).map_err(|_| bad())
    }
}

impl TryFrom<String> for LogSource {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<LogSource> for String {
    fn from(v: LogSource) -> Self {
        v.to_string()
    }
}

/// One log line. `at_unix_ms` is parsed from the line when the format allows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub text: String,
    pub at_unix_ms: Option<u64>,
}

/// Lines read from a log file plus the byte offset to continue from (for `--follow`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LogChunk {
    pub lines: Vec<LogLine>,
    pub next_offset: u64,
}

/// What to read from a log file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogQuery {
    /// Last N lines (after the `since` filter).
    pub max_lines: usize,
    /// Only lines at or after this instant (lines without a parsable timestamp are kept).
    pub since_unix_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrip() {
        for s in ["apache", "apache-error", "mysql-8.4", "mysql-5.7-error", "redis", "mailpit", "access:shop", "error:shop@fix-1", "php:shop"] {
            assert_eq!(s.parse::<LogSource>().unwrap().to_string(), s);
        }
        for bad in ["Nginx", "php:", "foo:shop", "mysql-8-error", "-error"] {
            assert!(bad.parse::<LogSource>().is_err(), "{bad}");
        }
    }

    #[test]
    fn files_live_under_logs_dir() {
        let paths = LampoPaths::new("C:\\cfg", "C:\\data");
        let site: SiteId = "shop@fix-1".parse().unwrap();
        assert_eq!(LogSource::SitePhp(site.clone()).file(&paths), PathBuf::from("C:\\data").join("logs").join("projects").join("shop@fix-1").join("php_errors.log"));
        assert_eq!(LogSource::for_site(&site, ["mysql-8.4".parse().unwrap()]).len(), 4);
    }
}
