//! Live Apache httpd builds from Apache Lounge (`https://www.apachelounge.com/download/`).
//!
//! Apache Lounge keeps only the newest build of each Visual Studio toolset online, so pinned
//! URLs rot; this source reads the download page, picks the Win64 httpd zip(s) and takes the
//! SHA-256 from the `<zip>.txt` checksum file published next to each binary.
//!
//! mod_fcgid is *not* served here: Apache Lounge publishes no checksum for modules, so its entry
//! is pinned in the embedded catalog with a SHA-256 computed from the file.

use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::package::{CatalogChannel, CatalogEntry, DownloadUrl, PackageId, PackageKind, PackagePayload, Sha256Digest};
use lampo_domain::ports::PackageCatalogSource;
use lampo_domain::vc_runtime::VcRuntime;
use lampo_domain::version::PackageVersion;
use lampo_domain::{PortError, PortResult};

use super::{TtlCache, hrefs};

pub const SITE_URL: &str = "https://www.apachelounge.com";
const TTL: Duration = Duration::from_secs(60 * 60);

pub struct ApacheLoungeSource {
    client: reqwest::Client,
    base_url: String,
    attempts: u32,
    cache: TtlCache,
}

impl ApacheLoungeSource {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client, base_url: SITE_URL.to_string(), attempts: 3, cache: TtlCache::new() }
    }

    /// Overrides the site root and disables retries (tests).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self.attempts = 1;
        self
    }

    async fn load(&self) -> PortResult<(Vec<CatalogEntry>, Duration)> {
        let page = crate::http::get_text(&self.client, &format!("{}/download/", self.base_url), self.attempts).await?;
        let builds = find_httpd_builds(&page);
        if builds.is_empty() {
            return Err(PortError::invalid_data("apachelounge.com download page", "no Win64 httpd build found"));
        }
        let mut entries = Vec::new();
        for build in builds {
            let txt_url = format!("{}{}.txt", self.base_url, build.path);
            let txt = match crate::http::get_text(&self.client, &txt_url, self.attempts).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(url = %txt_url, error = %e, "no checksum for Apache Lounge build; skipping it");
                    continue;
                }
            };
            let Some(sha256) = parse_sha256_txt(&txt) else {
                tracing::warn!(url = %txt_url, "checksum file without SHA-256; skipping the build");
                continue;
            };
            entries.push(CatalogEntry {
                id: PackageId::new(PackageKind::APACHE, build.version),
                url: DownloadUrl::new(format!("{SITE_URL}{}", build.path))?,
                sha256: Some(sha256),
                size_bytes: None,
                vc_runtime: Some(build.vc_runtime),
                channel: CatalogChannel::Live,
                end_of_life: false,
                payload: PackagePayload::Zip { strip_prefix: Some("Apache24".into()) },
            });
        }
        Ok((entries, TTL))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpdBuild {
    /// Site-relative path (`/download/VS18/binaries/httpd-2.4.71-261112-Win64-VS18.zip`).
    pub path: String,
    /// `2.4.71-261112` (httpd version + Apache Lounge build date).
    pub version: PackageVersion,
    pub vc_runtime: VcRuntime,
}

/// Win64 httpd zips linked from the download page, built with a toolset Lampo knows.
pub fn find_httpd_builds(html: &str) -> Vec<HttpdBuild> {
    let mut out: Vec<HttpdBuild> = Vec::new();
    for href in hrefs(html) {
        let Some(build) = parse_httpd_href(&href) else { continue };
        if !out.iter().any(|b| b.version == build.version) {
            out.push(build);
        }
    }
    out
}

fn parse_httpd_href(href: &str) -> Option<HttpdBuild> {
    // /download/VS18/binaries/httpd-2.4.71-261112-Win64-VS18.zip
    let path = href.strip_prefix(SITE_URL).unwrap_or(href);
    let rest = path.strip_prefix("/download/")?;
    let (toolset_dir, rest) = rest.split_once('/')?;
    let file = rest.strip_prefix("binaries/")?;
    if file.contains('/') {
        return None;
    }
    let stem = file.strip_prefix("httpd-")?.strip_suffix(".zip")?;
    let lower = stem.to_ascii_lowercase();
    let toolset = toolset_dir.to_ascii_lowercase();
    let (version, suffix) = lower.split_once("-win64-")?;
    if suffix != toolset {
        return None;
    }
    let vc_runtime = match toolset.as_str() {
        "vs16" => VcRuntime::Vs16,
        "vs17" => VcRuntime::Vs17,
        "vs18" => VcRuntime::Vs18,
        _ => return None, // unknown runtime requirement: do not offer it
    };
    Some(HttpdBuild { path: path.to_string(), version: PackageVersion::parse(version).ok()?, vc_runtime })
}

/// The SHA-256 of an Apache Lounge checksum file (`SHA256-Checksum for: <file>:\n<HEX>`).
pub fn parse_sha256_txt(text: &str) -> Option<Sha256Digest> {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("SHA256-Checksum") {
            return lines.map(str::trim).find(|l| !l.is_empty()).and_then(|l| Sha256Digest::parse_hex(l).ok());
        }
    }
    None
}

#[async_trait]
impl PackageCatalogSource for ApacheLoungeSource {
    fn name(&self) -> &str {
        "apachelounge.com"
    }

    fn kinds(&self) -> Vec<PackageKind> {
        vec![PackageKind::APACHE]
    }

    async fn fetch(&self, kind: &PackageKind) -> PortResult<Vec<CatalogEntry>> {
        if kind != &PackageKind::APACHE {
            return Ok(Vec::new());
        }
        self.cache.get_or_load(|| self.load()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{Reply, TestServer};

    const PAGE: &str = r#"
        <a href="/download/VS18/binaries/httpd-2.4.71-261112-Win64-VS18.zip">httpd-2.4.71-261112-Win64-VS18.zip</a>
        <a href="/download/VS18/binaries/httpd-2.4.71-261112-Win64-VS18.zip.txt">SHA</a>
        <a href="/download/VS18/binaries/httpd-2.4.71-261112-win32-vs18.zip">32 bit</a>
        <a href="/download/VS18/modules/mod_fcgid-2.3.10-win64-VS18.zip">mod_fcgid</a>
        <a href="/download/VS19/binaries/httpd-2.4.80-280101-Win64-VS19.zip">future toolset</a>"#;

    const TXT: &str = "Checksums created with GPGHash\n\nSHA1-Checksum for: httpd.zip:\n3493F205D220F13651AD564C48F51F43C080C5FC\n\nSHA256-Checksum for: httpd-2.4.71-261112-Win64-VS18.zip:\n44961A929DFF16B767B3ADBCEA55760A942A00239B2891DC55806BE34029E1FC\n\nSHA384-Checksum for: x:\nAB\n";

    #[test]
    fn finds_win64_builds_with_known_toolsets() {
        let builds = find_httpd_builds(PAGE);
        assert_eq!(builds.len(), 1);
        assert_eq!(builds[0].version.as_str(), "2.4.71-261112");
        assert_eq!(builds[0].vc_runtime, VcRuntime::Vs18);
    }

    #[test]
    fn parses_checksum_files() {
        assert_eq!(parse_sha256_txt(TXT).unwrap().to_string(), "44961a929dff16b767b3adbcea55760a942a00239b2891dc55806be34029e1fc");
        assert!(parse_sha256_txt("SHA1-Checksum for: x:\nABC").is_none());
    }

    #[tokio::test]
    async fn fetches_entries() {
        let server = TestServer::start(|req| match req.path.as_str() {
            "/download/" => Reply::text(PAGE),
            "/download/VS18/binaries/httpd-2.4.71-261112-Win64-VS18.zip.txt" => Reply::text(TXT),
            _ => Reply::status(404),
        })
        .await;
        let source = ApacheLoungeSource::new(reqwest::Client::new()).with_base_url(server.url(""));
        let entries = source.fetch(&PackageKind::APACHE).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url.as_str(), "https://www.apachelounge.com/download/VS18/binaries/httpd-2.4.71-261112-Win64-VS18.zip");
        assert!(source.fetch(&PackageKind::MOD_FCGID).await.unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn live_apache_lounge_page() {
        let source = ApacheLoungeSource::new(crate::http::build_client().unwrap());
        let entries = source.fetch(&PackageKind::APACHE).await.unwrap();
        assert!(!entries.is_empty());
        for e in &entries {
            println!("{} {} {:?} {}", e.id, e.url, e.vc_runtime, e.sha256.unwrap());
        }
    }
}
