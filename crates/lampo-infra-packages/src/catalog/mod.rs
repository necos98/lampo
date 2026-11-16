//! Catalog sources, one per origin, all behind `PackageCatalogSource`. Composition roots order
//! them live-first so live metadata (a moved URL) wins on duplicates, while the SHA-256 pinned in
//! the embedded catalog always wins over a live one (`lampo_domain::package::merge_catalogs`);
//! [`default_sources`] does exactly that.
//!
//! | Source                     | Origin                                   | Kinds |
//! |----------------------------|------------------------------------------|-------|
//! | [`php_net`]                | windows.php.net `releases.json` (+ disk cache + embedded snapshot) | php |
//! | [`composer`]               | getcomposer.org `/versions` + `.sha256sum` (+ disk cache) | composer |
//! | [`apache_lounge`]          | apachelounge.com download page + `.txt` checksums | apache |
//! | [`embedded`]               | `frozen.toml` compiled in                | all kinds, pinned |
//!
//! The live sources answer from a stale-while-revalidate cache ([`revalidating_cache`]): a
//! listing is served at once from what is known and refreshed from the network in background.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::PackageCatalogSource;

pub mod apache_lounge;
pub mod composer;
pub mod embedded;
pub mod php_net;
mod revalidating_cache;

/// Every source, live ones first, the embedded catalog last (offline fallback).
pub fn default_sources(client: reqwest::Client, paths: &LampoPaths) -> Vec<Arc<dyn PackageCatalogSource>> {
    let cache = cache_dir(paths);
    vec![
        Arc::new(php_net::PhpNetReleasesSource::new(client.clone()).with_cache_file(cache.join("windows.php.net-releases.json"))),
        Arc::new(composer::ComposerSource::new(client.clone()).with_cache_file(cache.join("getcomposer.org-versions.json"))),
        Arc::new(apache_lounge::ApacheLoungeSource::new(client)),
        Arc::new(embedded::EmbeddedCatalog::new()),
    ]
}

/// `%LOCALAPPDATA%\Lampo\packages\catalog-cache\`
pub fn cache_dir(paths: &LampoPaths) -> PathBuf {
    paths.packages_dir().join("catalog-cache")
}

/// Replaces `file` with `text` atomically (temporary sibling + rename), creating its directory.
/// A failure is logged, not returned: the cache file is only an offline fallback.
pub(crate) async fn write_cache_file(file: &Path, text: &str) {
    let write = async {
        if let Some(dir) = file.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let mut tmp = file.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        tokio::fs::write(&tmp, text).await?;
        tokio::fs::rename(&tmp, file).await
    };
    if let Err(e) = write.await {
        tracing::warn!(file = %file.display(), error = %e, "cannot write the catalog cache file");
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
