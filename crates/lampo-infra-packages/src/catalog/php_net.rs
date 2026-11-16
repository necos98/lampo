//! Live list of PHP releases from `https://windows.php.net/downloads/releases/releases.json`
//! (NTS x64 zips only; sha256 and VS toolset read from the JSON).
//!
//! * Served from a stale-while-revalidate cache ([`super::revalidating_cache`]), fresh for
//!   [`TTL`]: until the first refresh succeeds, listings get the cache file (when configured)
//!   or failing that the copy embedded at build time (`php_net_snapshot.json`), so the newest
//!   known patch of each supported series is listed at once and stays installable offline.
//! * Every successful download is written to the cache file.
//! * URLs point at `/downloads/releases/archives/`, which holds every build including the
//!   current ones, so an entry stays valid after the next patch release moves the file out of
//!   `/downloads/releases/`.
//! * `end_of_life` comes from php.net's published end-of-life dates ([`php_end_of_life`]),
//!   compared with today's date, so a series turns EOL on its own when its date passes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::package::{CatalogChannel, CatalogEntry, DownloadUrl, PackageId, PackageKind, PackagePayload, Sha256Digest};
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::PackageCatalogSource;
use lampo_domain::progress::CancelFlag;
use lampo_domain::vc_runtime::VcRuntime;
use lampo_domain::version::PackageVersion;
use lampo_domain::{PortError, PortResult};

use super::revalidating_cache::{LiveCatalog, RevalidatingCache};
use super::write_cache_file;
use crate::http::{FetchPolicy, get_text};

pub const RELEASES_JSON_URL: &str = "https://windows.php.net/downloads/releases/releases.json";
pub const ARCHIVES_BASE_URL: &str = "https://windows.php.net/downloads/releases/archives/";
/// `releases.json` as of the build (verified: every NTS x64 zip hash matched the file).
pub const SNAPSHOT_JSON: &str = include_str!("php_net_snapshot.json");

const TTL: Duration = Duration::from_secs(15 * 60);
const SOURCE_NAME: &str = "windows.php.net";

pub struct PhpNetReleasesSource {
    live: Arc<ReleasesJson>,
    cache: RevalidatingCache,
}

/// Where `releases.json` comes from and where its last good copy is kept.
#[derive(Clone)]
struct ReleasesJson {
    client: reqwest::Client,
    cache_file: Option<PathBuf>,
    url: String,
}

impl PhpNetReleasesSource {
    pub fn new(client: reqwest::Client) -> Self {
        let live = ReleasesJson { client, cache_file: None, url: RELEASES_JSON_URL.to_string() };
        Self { live: Arc::new(live), cache: RevalidatingCache::new(SOURCE_NAME, TTL) }
    }

    /// Keeps the last good `releases.json` in `file` as offline fallback.
    pub fn with_cache_file(mut self, file: impl Into<PathBuf>) -> Self {
        Arc::make_mut(&mut self.live).cache_file = Some(file.into());
        self
    }

    /// Overrides the list URL (tests).
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        Arc::make_mut(&mut self.live).url = url.into();
        self
    }
}

#[async_trait]
impl LiveCatalog for ReleasesJson {
    /// The cache file, else the embedded snapshot.
    async fn stored(&self) -> Option<Vec<CatalogEntry>> {
        let today = Date::today();
        if let Some(file) = &self.cache_file {
            if let Ok(text) = tokio::fs::read_to_string(file).await {
                match parse_releases(&text, today) {
                    Ok(entries) if !entries.is_empty() => return Some(entries),
                    _ => tracing::warn!(file = %file.display(), "cached releases.json is unusable; using the embedded snapshot"),
                }
            }
        }
        match parse_releases(SNAPSHOT_JSON, today) {
            Ok(entries) => Some(entries),
            Err(e) => {
                tracing::error!(error = %e, "the embedded releases.json snapshot is unusable");
                None
            }
        }
    }

    async fn fetch_live(&self, cancel: &CancelFlag) -> PortResult<Vec<CatalogEntry>> {
        let text = get_text(&self.client, &self.url, FetchPolicy::LIST, cancel).await?;
        let entries = parse_releases(&text, Date::today())?;
        if entries.is_empty() {
            return Err(PortError::invalid_data(SOURCE_NAME, "releases.json lists no NTS x64 build"));
        }
        if let Some(file) = &self.cache_file {
            write_cache_file(file, &text).await;
        }
        Ok(entries)
    }
}

#[async_trait]
impl PackageCatalogSource for PhpNetReleasesSource {
    fn name(&self) -> &str {
        SOURCE_NAME
    }

    fn kinds(&self) -> Vec<PackageKind> {
        vec![PackageKind::PHP]
    }

    async fn fetch(&self, kind: &PackageKind) -> PortResult<Vec<CatalogEntry>> {
        if kind != &PackageKind::PHP {
            return Ok(Vec::new());
        }
        self.cache.get(&self.live).await
    }
}

/// Parses `releases.json`: one NTS x64 entry per series. Malformed series are skipped (logged),
/// a malformed document is an error.
pub fn parse_releases(text: &str, today: Date) -> PortResult<Vec<CatalogEntry>> {
    let source = "windows.php.net releases.json";
    let doc: serde_json::Value = serde_json::from_str(text).map_err(|e| PortError::invalid_data(source, e))?;
    let series = doc.as_object().ok_or_else(|| PortError::invalid_data(source, "expected an object"))?;
    let mut out = Vec::new();
    for (key, release) in series {
        match parse_series(key, release, today) {
            Ok(Some(entry)) => out.push(entry),
            Ok(None) => {}
            Err(reason) => tracing::warn!(series = %key, reason, "skipping PHP series from releases.json"),
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn parse_series(key: &str, release: &serde_json::Value, today: Date) -> Result<Option<CatalogEntry>, String> {
    let Ok(minor) = PhpMinor::parse(key) else { return Ok(None) }; // older than 5.6 or not a series
    let version = release.get("version").and_then(|v| v.as_str()).ok_or("no version")?;
    let version = PackageVersion::parse(version).map_err(|e| e.to_string())?;
    if !minor.contains(&version) {
        return Err(format!("version {version} is not in series {minor}"));
    }
    let Some((build_key, build)) = release.as_object().and_then(|o| o.iter().find(|(k, _)| k.starts_with("nts-") && k.ends_with("-x64"))) else {
        return Ok(None);
    };
    let toolset = &build_key["nts-".len()..build_key.len() - "-x64".len()];
    let vc_runtime = serde_json::from_value::<VcRuntime>(serde_json::Value::String(toolset.to_ascii_lowercase())).ok();
    if vc_runtime.is_none() {
        tracing::warn!(toolset, series = %minor, "unknown Visual C++ toolset; runtime check disabled for this build");
    }
    let zip = build.get("zip").ok_or("no zip")?;
    let path = zip.get("path").and_then(|v| v.as_str()).ok_or("no zip path")?;
    if path.is_empty() || path.contains(['/', '\\']) || !path.ends_with(".zip") {
        return Err(format!("unexpected file name `{path}`"));
    }
    let sha256 = zip.get("sha256").and_then(|v| v.as_str()).ok_or("no sha256")?;
    Ok(Some(CatalogEntry {
        id: PackageId::new(PackageKind::PHP, version),
        url: DownloadUrl::new(format!("{ARCHIVES_BASE_URL}{path}")).map_err(|e| e.to_string())?,
        sha256: Some(Sha256Digest::parse_hex(sha256).map_err(|e| e.to_string())?),
        size_bytes: None, // releases.json only gives a rounded "32.6MB"
        vc_runtime,
        channel: CatalogChannel::Live,
        end_of_life: php_end_of_life(minor).is_some_and(|eol| today > eol),
        payload: PackagePayload::Zip { strip_prefix: None },
    }))
}

/// A calendar date (UTC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl Date {
    pub const fn new(year: i32, month: u8, day: u8) -> Self {
        Self { year, month, day }
    }

    pub fn today() -> Self {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        Self::from_days_since_epoch((secs / 86_400) as i64)
    }

    /// Howard Hinnant's `civil_from_days`.
    pub fn from_days_since_epoch(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Self { year, month, day }
    }
}

/// End of security support of each PHP series, from https://www.php.net/supported-versions.php
/// and https://www.php.net/eol.php. `None` = not known yet (a series newer than this table is
/// treated as supported).
pub fn php_end_of_life(minor: PhpMinor) -> Option<Date> {
    Some(match (minor.major(), minor.minor()) {
        (5, _) => Date::new(2018, 12, 31),
        (7, 0) => Date::new(2019, 1, 10),
        (7, 1) => Date::new(2019, 12, 1),
        (7, 2) => Date::new(2020, 11, 30),
        (7, 3) => Date::new(2021, 12, 6),
        (7, 4) => Date::new(2022, 11, 28),
        (8, 0) => Date::new(2023, 11, 26),
        (8, 1) => Date::new(2025, 12, 31),
        (8, 2) => Date::new(2026, 12, 31),
        (8, 3) => Date::new(2027, 12, 31),
        (8, 4) => Date::new(2028, 12, 31),
        (8, 5) => Date::new(2029, 12, 31),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{Reply, TestServer};

    const NOV_2026: Date = Date::new(2026, 11, 13);

    #[test]
    fn date_conversion() {
        assert_eq!(Date::from_days_since_epoch(0), Date::new(1970, 1, 1));
        assert_eq!(Date::from_days_since_epoch(20_405), Date::new(2025, 11, 13));
        assert_eq!(Date::from_days_since_epoch(11_016), Date::new(2000, 2, 29));
    }

    #[test]
    fn parses_the_embedded_snapshot() {
        let entries = parse_releases(SNAPSHOT_JSON, NOV_2026).unwrap();
        let find = |s: &str| entries.iter().find(|e| e.id.series().to_string() == s).unwrap_or_else(|| panic!("{s}"));
        let php83 = find("8.3");
        assert_eq!(php83.id.version.as_str(), "8.3.36-1");
        assert_eq!(php83.url.as_str(), "https://windows.php.net/downloads/releases/archives/php-8.3.36-1-nts-Win32-vs16-x64.zip");
        assert_eq!(php83.sha256.unwrap().to_string(), "1a097d52b7a68622cdc723073b61cc70caf79c941218ff65b2d8441dbc6b10d4");
        assert_eq!(php83.vc_runtime, Some(VcRuntime::Vs16));
        assert!(!php83.end_of_life);
        assert_eq!(find("8.4").vc_runtime, Some(VcRuntime::Vs17));
        assert_eq!(find("8.5").vc_runtime, Some(VcRuntime::Vs17));
        assert!(find("8.1").end_of_life);
        assert!(find("7.4").end_of_life);
        assert!(!find("8.2").end_of_life, "8.2 is supported until the end of 2026");
        assert!(entries.iter().all(|e| e.channel == CatalogChannel::Live && e.url.as_str().contains("-nts-") && e.url.as_str().ends_with("-x64.zip")));
    }

    #[test]
    fn series_become_eol_by_date() {
        let entries = parse_releases(SNAPSHOT_JSON, Date::new(2027, 1, 1)).unwrap();
        assert!(entries.iter().find(|e| e.id.series().to_string() == "8.2").unwrap().end_of_life);
    }

    #[test]
    fn skips_bad_series_but_keeps_good_ones() {
        let json = r#"{
            "8.3": {"version": "8.3.1", "nts-vs16-x64": {"zip": {"path": "php-8.3.1-nts-Win32-vs16-x64.zip", "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}},
            "8.4": {"version": "8.4.1", "nts-vs17-x64": {"zip": {"path": "../evil.zip", "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}},
            "8.5": {"version": "8.5.0", "ts-vs17-x64": {}},
            "9.0": {"version": "9.0.0", "nts-vs19-x64": {"zip": {"path": "php-9.0.0-nts-Win32-vs19-x64.zip", "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}}
        }"#;
        let entries = parse_releases(json, NOV_2026).unwrap();
        let versions: Vec<_> = entries.iter().map(|e| e.id.version.to_string()).collect();
        assert_eq!(versions, ["8.3.1", "9.0.0"]);
        assert_eq!(entries[1].vc_runtime, None, "unknown toolset");
        assert!(!entries[1].end_of_life, "unknown future series is supported");
        assert!(parse_releases("not json", NOV_2026).is_err());
    }

    const LIVE_JSON: &str = r#"{"8.3": {"version": "8.3.99", "nts-vs16-x64": {"zip": {"path": "php-8.3.99-nts-Win32-vs16-x64.zip", "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}}"#;
    const CACHED_JSON: &str = r#"{"8.3": {"version": "8.3.98", "nts-vs16-x64": {"zip": {"path": "php-8.3.98-nts-Win32-vs16-x64.zip", "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}}}"#;
    /// Longer than any prompt answer may take, shorter than the test timeout.
    const SLOW_SERVER: Duration = Duration::from_secs(3);
    const PROMPT: Duration = Duration::from_millis(500);

    fn versions(entries: &[CatalogEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.id.version.as_str()).collect()
    }

    #[tokio::test]
    async fn answers_with_the_snapshot_then_the_live_list_and_refreshes_the_cache_file() {
        let server = TestServer::start(move |_| Reply::text(LIVE_JSON)).await;
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache").join("releases.json");
        let live = PhpNetReleasesSource::new(reqwest::Client::new()).with_url(server.url("/releases.json")).with_cache_file(&cache);
        let first = live.fetch(&PackageKind::PHP).await.unwrap();
        assert!(versions(&first).contains(&"8.3.36-1"), "no cache file yet: the embedded snapshot");
        live.cache.settled().await;
        assert_eq!(versions(&live.fetch(&PackageKind::PHP).await.unwrap()), ["8.3.99"]);
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), LIVE_JSON, "a successful fetch refreshes the cache file");
        assert!(live.fetch(&PackageKind::APACHE).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn offline_keeps_the_cache_file_then_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("releases.json");
        std::fs::write(&cache, CACHED_JSON).unwrap();
        let offline = PhpNetReleasesSource::new(reqwest::Client::new()).with_url("http://127.0.0.1:9/releases.json").with_cache_file(&cache);
        assert_eq!(versions(&offline.fetch(&PackageKind::PHP).await.unwrap()), ["8.3.98"]);
        offline.cache.settled().await;
        assert_eq!(versions(&offline.fetch(&PackageKind::PHP).await.unwrap()), ["8.3.98"], "a failed refresh keeps the cache file's entries");

        let snapshot = PhpNetReleasesSource::new(reqwest::Client::new()).with_url("http://127.0.0.1:9/releases.json");
        assert!(versions(&snapshot.fetch(&PackageKind::PHP).await.unwrap()).contains(&"8.3.36-1"));
    }

    #[tokio::test]
    async fn a_slow_server_does_not_delay_the_listing() {
        let server = TestServer::start(move |_| Reply::text(LIVE_JSON).delayed(SLOW_SERVER)).await;
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("releases.json");
        std::fs::write(&cache, CACHED_JSON).unwrap();
        let source = PhpNetReleasesSource::new(reqwest::Client::new()).with_url(server.url("/releases.json")).with_cache_file(&cache);
        for _ in 0..3 {
            let entries = tokio::time::timeout(PROMPT, source.fetch(&PackageKind::PHP)).await.expect("answered from the cache file").unwrap();
            assert_eq!(versions(&entries), ["8.3.98"]);
        }
        source.cache.settled().await;
        assert_eq!(versions(&source.fetch(&PackageKind::PHP).await.unwrap()), ["8.3.99"]);
    }

    /// Live check against windows.php.net (run with `--ignored`).
    #[tokio::test]
    #[ignore = "network"]
    async fn live_releases_json_is_parseable() {
        let source = PhpNetReleasesSource::new(crate::http::build_client().unwrap());
        let entries = source.live.fetch_live(&CancelFlag::new()).await.unwrap();
        assert!(entries.iter().any(|e| !e.end_of_life), "at least one supported series");
        for e in &entries {
            assert!(e.sha256.is_some() && e.vc_runtime.is_some(), "{}", e.id);
            println!("{} {} {:?} eol={}", e.id, e.url, e.vc_runtime, e.end_of_life);
        }
    }
}
