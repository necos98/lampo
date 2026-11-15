//! Catalog sources, one per origin, all behind `PackageCatalogSource`. Composition roots order
//! them live-first so live data wins on duplicates (`lampo_domain::package::merge_catalogs`);
//! [`default_sources`] does exactly that.
//!
//! | Source                     | Origin                                   | Kinds |
//! |----------------------------|------------------------------------------|-------|
//! | [`php_net`]                | windows.php.net `releases.json` (+ disk cache + embedded snapshot) | php |
//! | [`composer`]               | getcomposer.org `/versions` + `.sha256sum` | composer |
//! | [`apache_lounge`]          | apachelounge.com download page + `.txt` checksums | apache |
//! | [`embedded`]               | `frozen.toml` compiled in                | all kinds, pinned |

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lampo_domain::package::CatalogEntry;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::PackageCatalogSource;

pub mod apache_lounge;
pub mod composer;
pub mod embedded;
pub mod php_net;

/// Every source, live ones first, the embedded catalog last (offline fallback).
pub fn default_sources(client: reqwest::Client, paths: &LampoPaths) -> Vec<Arc<dyn PackageCatalogSource>> {
    vec![
        Arc::new(php_net::PhpNetReleasesSource::new(client.clone()).with_cache_file(cache_dir(paths).join("windows.php.net-releases.json"))),
        Arc::new(composer::ComposerSource::new(client.clone())),
        Arc::new(apache_lounge::ApacheLoungeSource::new(client)),
        Arc::new(embedded::EmbeddedCatalog::new()),
    ]
}

/// `%LOCALAPPDATA%\Lampo\packages\catalog-cache\`
pub fn cache_dir(paths: &LampoPaths) -> PathBuf {
    paths.packages_dir().join("catalog-cache")
}

/// How long a failed live fetch is remembered, so an offline machine does not wait for the
/// network on every catalog listing.
const FAILURE_TTL: Duration = Duration::from_secs(60);

type CachedResult = lampo_domain::PortResult<Vec<CatalogEntry>>;

/// In-memory cache of one source's entries (or its last failure) with a time-to-live.
pub(crate) struct TtlCache {
    inner: tokio::sync::Mutex<Option<(Instant, Duration, CachedResult)>>,
}

impl TtlCache {
    pub(crate) fn new() -> Self {
        Self { inner: tokio::sync::Mutex::new(None) }
    }

    /// Returns the cached result if fresh, else runs `load` (serialised: concurrent callers wait
    /// for one fetch) and caches its result for the TTL it returns (failures for a minute).
    pub(crate) async fn get_or_load<F, Fut>(&self, load: F) -> CachedResult
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = lampo_domain::PortResult<(Vec<CatalogEntry>, Duration)>>,
    {
        let mut guard = self.inner.lock().await;
        if let Some((at, ttl, result)) = guard.as_ref() {
            if at.elapsed() < *ttl {
                return result.clone();
            }
        }
        let (result, ttl) = match load().await {
            Ok((entries, ttl)) => (Ok(entries), ttl),
            Err(e) => (Err(e), FAILURE_TTL),
        };
        *guard = Some((Instant::now(), ttl, result.clone()));
        result
    }
}

/// `href="…"` / `href='…'` values of an HTML page, in order.
pub(crate) fn hrefs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut pos = 0;
    while let Some(i) = lower[pos..].find("href=") {
        let start = pos + i + 5;
        let Some(quote) = html[start..].chars().next() else { break };
        if quote == '"' || quote == '\'' {
            if let Some(end) = html[start + 1..].find(quote) {
                out.push(html[start + 1..start + 1 + end].to_string());
                pos = start + 1 + end;
                continue;
            }
        }
        pos = start;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_hrefs() {
        let html = r#"<a href="/a.zip">A</a> <A HREF='/b.zip'>B</A> <a href=bare>"#;
        assert_eq!(hrefs(html), ["/a.zip", "/b.zip"]);
    }

    #[test]
    fn default_sources_put_live_first_and_embedded_last() {
        let paths = LampoPaths::new("C:\\cfg", "C:\\data");
        let sources = default_sources(reqwest::Client::new(), &paths);
        let names: Vec<_> = sources.iter().map(|s| s.name().to_string()).collect();
        assert_eq!(names, ["windows.php.net", "getcomposer.org", "apachelounge.com", "embedded"]);
    }
}
