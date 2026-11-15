//! Rebuilding the records of `installed.toml` from the slot directories, when the file no longer
//! parses: a slot name (`php-8.3`, `xdebug-3.5-php8.3`, see [`crate::registry::slot_key`]) tells
//! the kind, the series and the PHP minor. The exact version is not on disk, so it is recorded as
//! `<series>-unknown` (`8.3-unknown`): it sorts below every patch of the series, so the next
//! update in that series is offered, and installing it records the real version again.

use std::path::Path;

use lampo_domain::package::PackageKind;
use lampo_domain::php::PhpMinor;
use lampo_domain::version::{PackageVersion, VersionSeries};

use crate::registry::{PHP_MINOR_SEPARATOR, Record};

/// Build suffix of a version that could not be recovered.
const UNKNOWN_VERSION_SUFFIX: &str = "unknown";

/// One record per slot directory in `packages_dir`; anything else (lock files, leftovers of
/// interrupted operations, foreign folders) is skipped.
pub(crate) fn records_from_slots(packages_dir: &Path) -> std::io::Result<Vec<Record>> {
    let entries = match std::fs::read_dir(packages_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut records = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Ok(name) = entry.file_name().into_string() else { continue };
        match record_for_slot(&name) {
            Some(record) => records.push(record),
            None => tracing::debug!(dir = %name, "not a package slot; not recorded"),
        }
    }
    Ok(records)
}

/// The record of the slot directory `name`, with its version unknown.
fn record_for_slot(name: &str) -> Option<Record> {
    let (rest, php) = match name.rsplit_once(PHP_MINOR_SEPARATOR) {
        Some((rest, php)) => match PhpMinor::parse(php) {
            Ok(php) => (rest, Some(php)),
            Err(_) => (name, None),
        },
        None => (name, None),
    };
    let (kind, series) = rest.rsplit_once('-')?;
    let kind = PackageKind::new(kind).ok()?;
    let series = VersionSeries::parse(series).ok()?;
    let version = PackageVersion::parse(&format!("{series}-{UNKNOWN_VERSION_SUFFIX}")).ok()?;
    Some(Record { kind: kind.to_string(), version: version.to_string(), php: php.map(|p| p.to_string()), dir: name.to_string(), vc_runtime: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(name: &str) -> Option<(String, String, Option<String>)> {
        record_for_slot(name).map(|r| (r.kind, r.version, r.php))
    }

    #[test]
    fn slot_names_give_kind_series_and_php_minor() {
        let s = |kind: &str, version: &str, php: Option<&str>| Some((kind.to_string(), version.to_string(), php.map(str::to_string)));
        assert_eq!(summary("php-8.3"), s("php", "8.3-unknown", None));
        assert_eq!(summary("mod-fcgid-2.3"), s("mod-fcgid", "2.3-unknown", None));
        assert_eq!(summary("xdebug-3.5-php8.3"), s("xdebug", "3.5-unknown", Some("8.3")));
        assert_eq!(summary("php-redis-6.3-php8.4"), s("php-redis", "6.3-unknown", Some("8.4")));
        for other in ["php-8.3.old-12-34", "php-8.3.staging-1-2", "php", "Php-8.3", "php-8", "php-8.3.1", "-8.3", "readme"] {
            assert_eq!(summary(other), None, "{other}");
        }
    }

    #[test]
    fn only_slot_directories_are_recorded() {
        let dir = tempfile::tempdir().unwrap();
        for d in ["php-8.3", "redis-8.10", "php-8.3.old-12-34"] {
            std::fs::create_dir(dir.path().join(d)).unwrap();
        }
        std::fs::write(dir.path().join("php-8.4"), b"a file, not a slot").unwrap();
        let mut dirs: Vec<String> = records_from_slots(dir.path()).unwrap().into_iter().map(|r| r.dir).collect();
        dirs.sort();
        assert_eq!(dirs, ["php-8.3", "redis-8.10"]);
        assert!(records_from_slots(&dir.path().join("missing")).unwrap().is_empty());
    }
}
