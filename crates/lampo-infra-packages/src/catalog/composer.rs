//! Live Composer releases from getcomposer.org: `https://getcomposer.org/versions` lists the
//! latest stable and the 2.2 LTS (`stable` channel); each phar's SHA-256 is read from
//! `https://getcomposer.org/download/<version>/composer.phar.sha256sum`.

use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::package::{CatalogChannel, CatalogEntry, DownloadUrl, PackageId, PackageKind, PackagePayload, Sha256Digest};
use lampo_domain::ports::PackageCatalogSource;
use lampo_domain::version::PackageVersion;
use lampo_domain::{PortError, PortResult};
use serde::Deserialize;

use super::TtlCache;

pub const BASE_URL: &str = "https://getcomposer.org";
const TTL: Duration = Duration::from_secs(60 * 60);

pub struct ComposerSource {
    client: reqwest::Client,
    base_url: String,
    attempts: u32,
    cache: TtlCache,
}

impl ComposerSource {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client, base_url: BASE_URL.to_string(), attempts: 3, cache: TtlCache::new() }
    }

    /// Overrides the site root and disables retries (tests).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self.attempts = 1;
        self
    }

    async fn load(&self) -> PortResult<(Vec<CatalogEntry>, Duration)> {
        let text = crate::http::get_text(&self.client, &format!("{}/versions", self.base_url), self.attempts).await?;
        let mut entries = Vec::new();
        for release in parse_versions(&text)? {
            let sum_url = format!("{}/download/{}/composer.phar.sha256sum", self.base_url, release.version);
            let sum = crate::http::get_text(&self.client, &sum_url, self.attempts).await?;
            let hex = sum.split_whitespace().next().unwrap_or_default();
            let sha256 = Sha256Digest::parse_hex(hex).map_err(|e| PortError::invalid_data(sum_url.clone(), e))?;
            let version = PackageVersion::parse(&release.version).map_err(|e| PortError::invalid_data("getcomposer.org versions", e))?;
            entries.push(CatalogEntry {
                id: PackageId::new(PackageKind::COMPOSER, version),
                url: DownloadUrl::new(format!("{BASE_URL}{}", release.path))?,
                sha256: Some(sha256),
                size_bytes: None,
                vc_runtime: None,
                channel: CatalogChannel::Live,
                end_of_life: release.eol || release.maintenance.as_deref() == Some("eol"),
                payload: PackagePayload::SingleFile { file_name: "composer.phar".into() },
            });
        }
        Ok((entries, TTL))
    }
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
        "getcomposer.org"
    }

    fn kinds(&self) -> Vec<PackageKind> {
        vec![PackageKind::COMPOSER]
    }

    async fn fetch(&self, kind: &PackageKind) -> PortResult<Vec<CatalogEntry>> {
        if kind != &PackageKind::COMPOSER {
            return Ok(Vec::new());
        }
        self.cache.get_or_load(|| self.load()).await
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
        let entries = source.fetch(&PackageKind::COMPOSER).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].url.as_str(), "https://getcomposer.org/download/2.11.2/composer.phar");
        assert_eq!(entries[1].sha256.unwrap().to_string(), "241a3d13b7c6d35d970b1f69a000298b66943541302148a438a932f5dd44a9fc");
        assert!(source.fetch(&PackageKind::PHP).await.unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn live_composer_versions() {
        let source = ComposerSource::new(crate::http::build_client().unwrap());
        let entries = source.fetch(&PackageKind::COMPOSER).await.unwrap();
        assert!(entries.iter().any(|e| e.id.series().to_string() == "2.2"), "2.2 LTS listed");
        for e in &entries {
            println!("{} {} {}", e.id, e.url, e.sha256.unwrap());
        }
    }
}
