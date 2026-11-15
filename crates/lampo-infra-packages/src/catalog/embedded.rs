//! Entries compiled into the binary from `frozen.toml`: EOL PHP builds (5.6–8.1), Apache
//! Lounge + mod_fcgid, MySQL 8.4/5.7 zips, Redis for Windows, Mailpit, Composer stable + 2.2 LTS phar,
//! Xdebug and phpredis DLLs pinned per PHP minor. Every entry has a SHA-256.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use async_trait::async_trait;
use lampo_domain::package::{CatalogChannel, CatalogEntry, DownloadUrl, PackageId, PackageKind, PackagePayload, Sha256Digest};
use lampo_domain::php::PhpMinor;
use lampo_domain::ports::PackageCatalogSource;
use lampo_domain::vc_runtime::VcRuntime;
use lampo_domain::version::PackageVersion;
use lampo_domain::{DomainError, PortError, PortResult};
use serde::Deserialize;

/// The embedded data. Schema documented at the top of the file.
pub const FROZEN_CATALOG_TOML: &str = include_str!("frozen.toml");

pub struct EmbeddedCatalog {
    /// Parsed lazily on first use (a parse failure is a build defect: covered by a unit test).
    entries: OnceLock<Result<Vec<CatalogEntry>, PortError>>,
}

impl EmbeddedCatalog {
    /// Does no work; composition roots can call it unconditionally.
    pub fn new() -> Self {
        Self { entries: OnceLock::new() }
    }

    /// All embedded entries.
    pub fn entries(&self) -> PortResult<&[CatalogEntry]> {
        self.entries.get_or_init(|| parse(FROZEN_CATALOG_TOML)).as_deref().map_err(Clone::clone)
    }
}

impl Default for EmbeddedCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageCatalogSource for EmbeddedCatalog {
    fn name(&self) -> &str {
        "embedded"
    }

    fn kinds(&self) -> Vec<PackageKind> {
        let kinds: BTreeSet<PackageKind> = self.entries().unwrap_or_default().iter().map(|e| e.id.kind.clone()).collect();
        kinds.into_iter().collect()
    }

    async fn fetch(&self, kind: &PackageKind) -> PortResult<Vec<CatalogEntry>> {
        Ok(self.entries()?.iter().filter(|e| &e.id.kind == kind).cloned().collect())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenFile {
    #[serde(default)]
    entry: Vec<FrozenEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenEntry {
    kind: String,
    version: String,
    php: Option<String>,
    url: String,
    sha256: String,
    size_bytes: Option<u64>,
    vc_runtime: Option<VcRuntime>,
    #[serde(default)]
    end_of_life: bool,
    payload: PackagePayload,
}

impl FrozenEntry {
    fn into_entry(self) -> Result<CatalogEntry, DomainError> {
        let kind = PackageKind::new(self.kind)?;
        let version = PackageVersion::parse(&self.version)?;
        let php = self.php.as_deref().map(PhpMinor::parse).transpose()?;
        Ok(CatalogEntry {
            id: PackageId { kind, version, php },
            url: DownloadUrl::new(self.url)?,
            sha256: Some(Sha256Digest::parse_hex(&self.sha256)?),
            size_bytes: self.size_bytes,
            vc_runtime: self.vc_runtime,
            channel: CatalogChannel::Frozen,
            end_of_life: self.end_of_life,
            payload: self.payload,
        })
    }
}

/// Parses catalog TOML in the `frozen.toml` schema.
pub fn parse(text: &str) -> PortResult<Vec<CatalogEntry>> {
    let file: FrozenFile = toml::from_str(text).map_err(|e| PortError::invalid_data("embedded catalog", e))?;
    let mut out = Vec::with_capacity(file.entry.len());
    let mut seen = BTreeSet::new();
    for (i, raw) in file.entry.into_iter().enumerate() {
        let entry = raw.into_entry().map_err(|e| PortError::invalid_data("embedded catalog", format!("entry #{}: {e}", i + 1)))?;
        if !seen.insert(entry.id.clone()) {
            return Err(PortError::invalid_data("embedded catalog", format!("duplicate entry {}", entry.id)));
        }
        out.push(entry);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use lampo_domain::package::latest;

    use super::*;

    fn catalog() -> Vec<CatalogEntry> {
        EmbeddedCatalog::new().entries().unwrap().to_vec()
    }

    fn php(s: &str) -> PhpMinor {
        PhpMinor::parse(s).unwrap()
    }

    #[test]
    fn embedded_catalog_parses_and_every_entry_is_pinned() {
        let entries = catalog();
        assert!(entries.len() >= 40, "{}", entries.len());
        for e in &entries {
            assert!(e.sha256.is_some(), "{}", e.id);
            assert_eq!(e.channel, CatalogChannel::Frozen);
            assert!(!e.url.as_str().contains("github.com/lampo"), "nothing hosted by Lampo: {}", e.url);
        }
    }

    #[test]
    fn covers_every_php_series_from_5_6() {
        let entries = catalog();
        for (minor, vc) in [
            ("5.6", VcRuntime::Vc11),
            ("7.0", VcRuntime::Vc14),
            ("7.1", VcRuntime::Vc14),
            ("7.2", VcRuntime::Vc15),
            ("7.3", VcRuntime::Vc15),
            ("7.4", VcRuntime::Vc15),
            ("8.0", VcRuntime::Vs16),
            ("8.1", VcRuntime::Vs16),
        ] {
            let m = php(minor);
            let e = entries.iter().find(|e| e.id.kind == PackageKind::PHP && e.id.series() == m.as_series()).unwrap_or_else(|| panic!("PHP {minor}"));
            assert_eq!(e.vc_runtime, Some(vc), "PHP {minor}");
            assert_eq!(e.vc_runtime, Some(VcRuntime::for_php(m)), "domain mapping agrees for {minor}");
            assert!(e.end_of_life);
        }
    }

    #[test]
    fn extensions_exist_for_every_php_minor() {
        let entries = catalog();
        for minor in ["5.6", "7.0", "7.1", "7.2", "7.3", "7.4", "8.0", "8.1", "8.2", "8.3", "8.4", "8.5"] {
            for kind in [PackageKind::XDEBUG, PackageKind::PHP_REDIS] {
                let e = latest(&entries, &kind, Some(php(minor))).unwrap_or_else(|| panic!("{kind} for PHP {minor}"));
                assert_eq!(e.vc_runtime, Some(VcRuntime::for_php(php(minor))), "{kind} {minor} is built with the PHP toolset");
            }
        }
    }

    #[test]
    fn has_the_service_packages() {
        let entries = catalog();
        for kind in [PackageKind::APACHE, PackageKind::MOD_FCGID, PackageKind::MYSQL, PackageKind::REDIS, PackageKind::MAILPIT, PackageKind::COMPOSER] {
            assert!(latest(&entries, &kind, None).is_some(), "{kind}");
        }
        let mysql: Vec<_> = entries.iter().filter(|e| e.id.kind == PackageKind::MYSQL).map(|e| e.id.series().to_string()).collect();
        assert!(mysql.contains(&"8.4".to_string()) && mysql.contains(&"5.7".to_string()));
        let composer: Vec<_> = entries.iter().filter(|e| e.id.kind == PackageKind::COMPOSER).map(|e| e.id.series().to_string()).collect();
        assert!(composer.contains(&"2.2".to_string()), "2.2 LTS for PHP < 7.2");
        assert!(matches!(&latest(&entries, &PackageKind::APACHE, None).unwrap().payload, PackagePayload::Zip { strip_prefix: Some(p) } if p == "Apache24"));
    }

    #[tokio::test]
    async fn fetch_filters_by_kind_and_kinds_lists_them() {
        let c = EmbeddedCatalog::new();
        assert!(c.kinds().contains(&PackageKind::PHP));
        let redis = c.fetch(&PackageKind::REDIS).await.unwrap();
        assert!(!redis.is_empty() && redis.iter().all(|e| e.id.kind == PackageKind::REDIS));
        assert!(c.fetch(&PackageKind::new("unknown-kind").unwrap()).await.unwrap().is_empty());
    }

    #[test]
    fn rejects_entries_without_checksum_or_with_bad_data() {
        let base = "kind = \"php\"\nversion = \"7.4.33\"\nurl = \"https://example.invalid/x.zip\"\npayload = { type = \"zip\" }\n";
        assert!(parse(&format!("[[entry]]\n{base}")).is_err(), "sha256 is required");
        assert!(parse(&format!("[[entry]]\n{base}sha256 = \"abc\"\n")).is_err());
        let ok = format!("[[entry]]\n{base}sha256 = \"{}\"\n", "a".repeat(64));
        assert_eq!(parse(&ok).unwrap().len(), 1);
        assert!(parse(&format!("{ok}{ok}")).is_err(), "duplicates are refused");
        assert!(parse(&ok.replace("https://", "http://")).is_err());
    }
}
