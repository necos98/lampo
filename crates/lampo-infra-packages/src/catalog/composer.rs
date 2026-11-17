//! Live Composer releases from getcomposer.org: `https://getcomposer.org/versions` lists the
//! latest stable and the 2.2 LTS (`stable` channel); each phar's SHA-256 is read from
//! `https://getcomposer.org/download/<version>/composer.phar.sha256sum` (all in parallel).
//!
//! A release whose checksum cannot be downloaded is skipped, the others are still listed. The
//! last valid `versions` document is kept in the cache file (when configured) together with the
//! checksums, and answers listings until a refresh succeeds ([`super::revalidating_cache`]);
//! without it (first run) the pinned Composer entries of the embedded catalog answer instead.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::future::join_all;
use lampo_domain::package::{CatalogChannel, CatalogEntry, DownloadUrl, PackageId, PackageKind, PackagePayload, Sha256Digest};
use lampo_domain::ports::PackageCatalogSource;
use lampo_domain::progress::CancelFlag;
use lampo_domain::version::PackageVersion;
use lampo_domain::{PortError, PortResult};
use serde::{Deserialize, Serialize};

use super::embedded::pinned_entries;
use super::revalidating_cache::{LiveCatalog, RevalidatingCache};
use super::write_cache_file;
use crate::http::{FetchPolicy, get_text};

pub const BASE_URL: &str = "https://getcomposer.org";
const TTL: Duration = Duration::from_secs(60 * 60);
const SOURCE_NAME: &str = "getcomposer.org";

pub struct ComposerSource {
    live: Arc<ComposerSite>,
    cache: RevalidatingCache,
}

/// Where the releases come from and where the last good list is kept.
#[derive(Clone)]
struct ComposerSite {
    client: reqwest::Client,
    base_url: String,
    checksum_policy: FetchPolicy,
    cache_file: Option<PathBuf>,
}

/// The cache file: the last valid `/versions` document and the SHA-256 of each listed phar
/// that could be read (releases without one are not listed).
#[derive(Debug, Serialize, Deserialize)]
struct CachedVersions {
    versions: String,
    sha256: BTreeMap<String, Sha256Digest>,
}

impl ComposerSource {
    pub fn new(client: reqwest::Client) -> Self {
        let live = ComposerSite { client, base_url: BASE_URL.to_string(), checksum_policy: FetchPolicy::CHECKSUM, cache_file: None };
        Self { live: Arc::new(live), cache: RevalidatingCache::new(SOURCE_NAME, TTL) }
    }

    /// Keeps the last good release list (with checksums) in `file` as offline fallback.
    pub fn with_cache_file(mut self, file: impl Into<PathBuf>) -> Self {
        Arc::make_mut(&mut self.live).cache_file = Some(file.into());
        self
    }

    /// Overrides the site root and disables retries (tests).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        let live = Arc::make_mut(&mut self.live);
        live.base_url = base_url.into();
        live.checksum_policy = FetchPolicy { attempts: 1, ..FetchPolicy::CHECKSUM };
        self
    }
}

impl ComposerSite {
    /// The phar's published SHA-256, `None` (logged) when it cannot be read.
    async fn checksum(&self, version: &str, cancel: &CancelFlag) -> Option<Sha256Digest> {
        let url = format!("{}/download/{version}/composer.phar.sha256sum", self.base_url);
        let digest = get_text(&self.client, &url, self.checksum_policy, cancel)
            .await
            .and_then(|sum| Sha256Digest::parse_hex(sum.split_whitespace().next().unwrap_or_default()).map_err(|e| PortError::invalid_data(url.as_str(), e)));
        match digest {
            Ok(digest) => Some(digest),
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "no checksum for Composer release; skipping it");
                None
            }
        }
    }

    /// The releases kept in the cache file, `None` when there is none or it is unusable (logged).
    async fn cached_releases(&self) -> Option<Vec<CatalogEntry>> {
        let file = self.cache_file.as_ref()?;
        let text = tokio::fs::read_to_string(file).await.ok()?;
        let cached = serde_json::from_str::<CachedVersions>(&text).map_err(|e| PortError::invalid_data(file.display().to_string(), e));
        match cached.and_then(|cached| entries(&parse_versions(&cached.versions)?, &cached.sha256)) {
            Ok(entries) if !entries.is_empty() => Some(entries),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(file = %file.display(), error = %e, "cached Composer versions are unusable");
                None
            }
        }
    }
}

#[async_trait]
impl LiveCatalog for ComposerSite {
    /// The cache file, else the pinned Composer entries of the embedded catalog.
    async fn stored(&self) -> Option<Vec<CatalogEntry>> {
        match self.cached_releases().await {
            Some(entries) => Some(entries),
            None => pinned_entries(&PackageKind::COMPOSER),
        }
    }

    async fn fetch_live(&self, cancel: &CancelFlag) -> PortResult<Vec<CatalogEntry>> {
        let versions = get_text(&self.client, &format!("{}/versions", self.base_url), FetchPolicy::LIST, cancel).await?;
        let releases = parse_versions(&versions)?;
        let digests = join_all(releases.iter().map(|r| self.checksum(&r.version, cancel))).await;
        cancel.check()?;
        let sha256: BTreeMap<String, Sha256Digest> = releases.iter().zip(digests).filter_map(|(r, digest)| Some((r.version.clone(), digest?))).collect();
        let entries = entries(&releases, &sha256)?;
        if entries.is_empty() {
            return Err(PortError::Network(format!("{SOURCE_NAME}: no release checksum could be read")));
        }
        if let Some(file) = &self.cache_file {
            match serde_json::to_string(&CachedVersions { versions, sha256 }) {
                Ok(text) => write_cache_file(file, &text).await,
                Err(e) => tracing::warn!(error = %e, "cannot serialise the Composer versions cache"),
            }
        }
        Ok(entries)
    }
}

/// Catalog entries of the releases with a known SHA-256 (the others are left out).
fn entries(releases: &[ComposerRelease], sha256: &BTreeMap<String, Sha256Digest>) -> PortResult<Vec<CatalogEntry>> {
    let mut out = Vec::new();
    for release in releases {
        let Some(sha256) = sha256.get(&release.version) else { continue };
        let version = PackageVersion::parse(&release.version).map_err(|e| PortError::invalid_data("getcomposer.org versions", e))?;
        out.push(CatalogEntry {
            id: PackageId::new(PackageKind::COMPOSER, version),
            url: DownloadUrl::new(format!("{BASE_URL}{}", release.path))?,
            sha256: Some(*sha256),
            size_bytes: None,
            vc_runtime: None,
            channel: CatalogChannel::Live,
            end_of_life: release.eol || release.maintenance.as_deref() == Some("eol"),
            payload: PackagePayload::SingleFile { file_name: "composer.phar".into() },
        });
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
pub struct ComposerRelease {
    pub path: String,
    pub version: String,
    #[serde(default)]
    pub lts: bool,
    #[serde(default)]
    pub eol: bool,
    #[serde(default)]
    pub maintenance: Option<String>,
}

/// The releases of the `stable` channel (latest stable + current LTS), validated.
pub fn parse_versions(text: &str) -> PortResult<Vec<ComposerRelease>> {
    #[derive(Deserialize)]
    struct Versions {
        stable: Vec<ComposerRelease>,
    }
    let source = "getcomposer.org versions";
    let versions: Versions = serde_json::from_str(text).map_err(|e| PortError::invalid_data(source, e))?;
    let mut out = Vec::new();
    for release in versions.stable {
        let expected = format!("/download/{}/composer.phar", release.version);
        if release.path != expected {
            return Err(PortError::invalid_data(source, format!("unexpected path `{}`", release.path)));
        }
        if !out.iter().any(|r: &ComposerRelease| r.version == release.version) {
            out.push(release);
        }
    }
    if out.is_empty() {
        return Err(PortError::invalid_data(source, "no stable release listed"));
    }
    Ok(out)
}

#[async_trait]
impl PackageCatalogSource for ComposerSource {
    fn name(&self) -> &str {
        SOURCE_NAME
    }

    fn kinds(&self) -> Vec<PackageKind> {
        vec![PackageKind::COMPOSER]
    }

    async fn fetch(&self, kind: &PackageKind) -> PortResult<Vec<CatalogEntry>> {
        if kind != &PackageKind::COMPOSER {
            return Ok(Vec::new());
        }
        self.cache.get(&self.live).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{Reply, TestServer};

    const VERSIONS: &str = r#"{
        "stable": [
            {"path": "/download/2.11.2/composer.phar", "version": "2.11.2", "min-php": 70205, "lts": false, "maintenance": "bugfix"},
            {"path": "/download/2.2.32/composer.phar", "version": "2.2.32", "min-php": 50300, "lts": true, "maintenance": "critical-security"}
        ],
        "preview": [], "snapshot": []
    }"#;

    #[test]
    fn parses_stable_and_lts() {
        let r = parse_versions(VERSIONS).unwrap();
        assert_eq!(r.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(), ["2.11.2", "2.2.32"]);
        assert!(r[1].lts);
        assert!(parse_versions(&VERSIONS.replace("/download/2.2.32/composer.phar", "https://evil.example/c.phar")).is_err());
    }

    #[tokio::test]
    async fn fetches_entries_with_checksums() {
        let server = TestServer::start(|req| match req.path.as_str() {
            "/versions" => Reply::text(VERSIONS),
            "/download/2.11.2/composer.phar.sha256sum" => Reply::text("dd158c0afd41d15fac89e1b4e47b5d42911fe316c3687cd5edac98319b25d028  composer.phar\n"),
            "/download/2.2.32/composer.phar.sha256sum" => Reply::text("241a3d13b7c6d35d970b1f69a000298b66943541302148a438a932f5dd44a9fc  composer.phar\n"),
            _ => Reply::status(404),
        })
        .await;
        let source = ComposerSource::new(reqwest::Client::new()).with_base_url(server.url(""));
        let entries = source.live.fetch_live(&CancelFlag::new()).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].url.as_str(), "https://getcomposer.org/download/2.11.2/composer.phar");
        assert_eq!(entries[1].sha256.unwrap().to_string(), "241a3d13b7c6d35d970b1f69a000298b66943541302148a438a932f5dd44a9fc");
        assert!(source.fetch(&PackageKind::PHP).await.unwrap().is_empty());
    }

    const SUM_2_11_2: &str = "dd158c0afd41d15fac89e1b4e47b5d42911fe316c3687cd5edac98319b25d028";

    /// `/versions` and only the 2.11.2 checksum (the 2.2.32 one is missing), after `delay`.
    async fn site_without_the_lts_checksum(delay: Duration) -> TestServer {
        TestServer::start(move |req| {
            let reply = match req.path.as_str() {
                "/versions" => Reply::text(VERSIONS),
                "/download/2.11.2/composer.phar.sha256sum" => Reply::text(&format!("{SUM_2_11_2}  composer.phar\n")),
                _ => Reply::status(404),
            };
            reply.delayed(delay)
        })
        .await
    }

    fn versions(entries: &[CatalogEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.id.version.as_str()).collect()
    }

    #[tokio::test]
    async fn skips_a_release_without_checksum_and_keeps_the_list_on_disk() {
        let server = site_without_the_lts_checksum(Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache").join("versions.json");
        let source = ComposerSource::new(reqwest::Client::new()).with_base_url(server.url("")).with_cache_file(&cache);
        assert_eq!(versions(&source.live.fetch_live(&CancelFlag::new()).await.unwrap()), ["2.11.2"]);

        let offline = ComposerSource::new(reqwest::Client::new()).with_base_url("http://127.0.0.1:9").with_cache_file(&cache);
        let entries = offline.fetch(&PackageKind::COMPOSER).await.unwrap();
        assert_eq!(versions(&entries), ["2.11.2"], "served from the cache file");
        assert_eq!(entries[0].sha256.unwrap().to_string(), SUM_2_11_2);
    }

    #[tokio::test]
    async fn every_checksum_missing_is_an_error_and_writes_nothing() {
        let server = TestServer::start(|req| if req.path == "/versions" { Reply::text(VERSIONS) } else { Reply::status(404) }).await;
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("versions.json");
        let source = ComposerSource::new(reqwest::Client::new()).with_base_url(server.url("")).with_cache_file(&cache);
        assert!(source.live.fetch_live(&CancelFlag::new()).await.is_err());
        assert!(!cache.exists());
    }

    #[tokio::test]
    async fn a_slow_site_does_not_delay_the_listing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("versions.json");
        let sha256 = BTreeMap::from([("2.2.32".to_string(), Sha256Digest::parse_hex(&"ab".repeat(32)).unwrap())]);
        std::fs::write(&cache, serde_json::to_string(&CachedVersions { versions: VERSIONS.into(), sha256 }).unwrap()).unwrap();
        let server = site_without_the_lts_checksum(Duration::from_secs(2)).await;
        let source = ComposerSource::new(reqwest::Client::new()).with_base_url(server.url("")).with_cache_file(&cache);
        let cached = tokio::time::timeout(Duration::from_millis(500), source.fetch(&PackageKind::COMPOSER)).await.expect("answered from the cache file").unwrap();
        assert_eq!(versions(&cached), ["2.2.32"]);
        source.cache.settled().await;
        assert_eq!(versions(&source.fetch(&PackageKind::COMPOSER).await.unwrap()), ["2.11.2"]);
    }

    #[tokio::test]
    async fn without_a_cache_file_a_slow_site_answers_with_the_pinned_releases() {
        let dir = tempfile::tempdir().unwrap();
        let server = site_without_the_lts_checksum(Duration::from_secs(2)).await;
        let source = ComposerSource::new(reqwest::Client::new()).with_base_url(server.url("")).with_cache_file(dir.path().join("versions.json"));
        let pinned = tokio::time::timeout(Duration::from_millis(500), source.fetch(&PackageKind::COMPOSER)).await.expect("answered with the pinned releases").unwrap();
        assert!(!pinned.is_empty() && pinned.iter().all(|e| e.id.kind == PackageKind::COMPOSER && e.channel == CatalogChannel::Frozen), "{pinned:?}");

        source.cache.settled().await;
        let live = source.fetch(&PackageKind::COMPOSER).await.unwrap();
        assert_eq!(live.iter().map(|e| (e.id.version.as_str(), e.channel)).collect::<Vec<_>>(), [("2.11.2", CatalogChannel::Live)]);
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn live_composer_versions() {
        let source = ComposerSource::new(crate::http::build_client().unwrap());
        let entries = source.live.fetch_live(&CancelFlag::new()).await.unwrap();
        assert!(entries.iter().any(|e| e.id.series().to_string() == "2.2"), "2.2 LTS listed");
        for e in &entries {
            println!("{} {} {}", e.id, e.url, e.sha256.unwrap());
        }
    }
}
