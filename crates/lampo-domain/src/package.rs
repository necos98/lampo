//! Downloadable packages: Apache, PHP, MySQL, Redis, Mailpit, Composer and pinned PHP add-on extensions.
//!
//! A package *kind* is an open identifier ([`PackageKind`]), not a closed enum: a new kind is
//! added by registering a [`crate::ports::PackageCatalogSource`] that serves it (and, if needed,
//! a [`crate::ports::PackageInstallHook`], and its file layout in `lampo-infra-packages`) — no
//! existing `match` has to change.

use std::borrow::Cow;
use std::fmt;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::php::PhpMinor;
use crate::vc_runtime::VcRuntime;
use crate::version::{PackageVersion, VersionSeries};

/// Identifier of a kind of package (`php`, `apache`, `xdebug`…): lowercase letters, digits, dashes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PackageKind(Cow<'static, str>);

impl PackageKind {
    /// Apache httpd (Apache Lounge build, VS17 x64).
    pub const APACHE: PackageKind = PackageKind(Cow::Borrowed("apache"));
    /// `mod_fcgid` for Apache — distributed by Apache Lounge as a separate download.
    pub const MOD_FCGID: PackageKind = PackageKind(Cow::Borrowed("mod-fcgid"));
    /// PHP NTS x64.
    pub const PHP: PackageKind = PackageKind(Cow::Borrowed("php"));
    pub const MYSQL: PackageKind = PackageKind(Cow::Borrowed("mysql"));
    pub const REDIS: PackageKind = PackageKind(Cow::Borrowed("redis"));
    /// Mailpit (official Windows amd64 release).
    pub const MAILPIT: PackageKind = PackageKind(Cow::Borrowed("mailpit"));
    pub const COMPOSER: PackageKind = PackageKind(Cow::Borrowed("composer"));
    /// Xdebug, pinned per PHP minor.
    pub const XDEBUG: PackageKind = PackageKind(Cow::Borrowed("xdebug"));
    /// The phpredis extension, pinned per PHP minor.
    pub const PHP_REDIS: PackageKind = PackageKind(Cow::Borrowed("php-redis"));

    pub fn new(id: impl Into<String>) -> Result<Self, DomainError> {
        let id: String = id.into();
        let ok = !id.is_empty() && id.len() <= 32 && id.as_bytes()[0].is_ascii_lowercase() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if ok { Ok(Self(Cow::Owned(id))) } else { Err(DomainError::InvalidPackageKind(id)) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for PackageKind {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<PackageKind> for String {
    fn from(v: PackageKind) -> Self {
        v.0.into_owned()
    }
}

/// Identity of one installable artifact.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PackageId {
    pub kind: PackageKind,
    pub version: PackageVersion,
    /// For PHP add-on extensions (Xdebug, phpredis): the PHP minor the binary is built for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub php: Option<PhpMinor>,
}

impl PackageId {
    pub fn new(kind: PackageKind, version: PackageVersion) -> Self {
        Self { kind, version, php: None }
    }

    pub fn for_php(kind: PackageKind, version: PackageVersion, php: PhpMinor) -> Self {
        Self { kind, version, php: Some(php) }
    }

    pub fn series(&self) -> VersionSeries {
        self.version.series()
    }

    /// Stable, filesystem-safe key (`php-8.3.14`, `xdebug-3.3.2-php8.3`), used for install dirs.
    pub fn key(&self) -> String {
        match self.php {
            Some(php) => format!("{}-{}-php{}", self.kind, self.version, php),
            None => format!("{}-{}", self.kind, self.version),
        }
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key())
    }
}

/// A SHA-256 digest (32 bytes), parsed from / displayed as lowercase hex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn parse_hex(hex: &str) -> Result<Self, DomainError> {
        let h = hex.trim();
        let err = || DomainError::InvalidSha256(hex.to_string());
        if h.len() != 64 {
            return Err(err());
        }
        let mut out = [0u8; 32];
        for (i, chunk) in h.as_bytes().chunks(2).enumerate() {
            let s = std::str::from_utf8(chunk).map_err(|_| err())?;
            out[i] = u8::from_str_radix(s, 16).map_err(|_| err())?;
        }
        Ok(Self(out))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl TryFrom<String> for Sha256Digest {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse_hex(&s)
    }
}

impl From<Sha256Digest> for String {
    fn from(v: Sha256Digest) -> Self {
        v.to_string()
    }
}

/// An `https://` download URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DownloadUrl(String);

impl DownloadUrl {
    pub fn new(url: impl Into<String>) -> Result<Self, DomainError> {
        let url: String = url.into();
        let ok = url.len() > "https://".len() && url.get(..8).is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://")) && !url.contains(char::is_whitespace);
        if ok { Ok(Self(url)) } else { Err(DomainError::InvalidDownloadUrl(url)) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Last path segment, used as the downloaded file name.
    pub fn file_name(&self) -> &str {
        let path = self.0.split(['?', '#']).next().unwrap_or(&self.0);
        path.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("download")
    }
}

impl fmt::Display for DownloadUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for DownloadUrl {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<DownloadUrl> for String {
    fn from(v: DownloadUrl) -> Self {
        v.0
    }
}

/// Where a catalog entry comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatalogChannel {
    /// Embedded in the app binary (EOL/frozen versions and pinned third-party packages).
    Frozen,
    /// Read live from the vendor (windows.php.net `releases.json`).
    Live,
}

/// How the downloaded payload becomes an installed package.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PackagePayload {
    /// A zip archive. `strip_prefix` removes a top-level folder (Apache Lounge zips contain `Apache24/`).
    Zip {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        strip_prefix: Option<String>,
    },
    /// A single file copied as-is into the install dir (`composer.phar`, `php_xdebug.dll`).
    SingleFile { file_name: String },
}

/// One installable version as advertised by a catalog source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: PackageId,
    pub url: DownloadUrl,
    /// Expected digest. Frozen entries always have one; a live entry without one is refused
    /// by the install use case unless the source is trusted to provide it later.
    pub sha256: Option<Sha256Digest>,
    pub size_bytes: Option<u64>,
    /// Visual C++ runtime the binaries need, if any.
    pub vc_runtime: Option<VcRuntime>,
    pub channel: CatalogChannel,
    /// Upstream no longer supports this series.
    pub end_of_life: bool,
    pub payload: PackagePayload,
}

/// What a file inside an installed package is *for*. Consumers ask an [`InstalledPackage`] for
/// a role instead of joining file names onto its install dir: where each kind keeps its files is
/// known only to the registry adapter (`lampo-infra-packages`), which fills
/// [`InstalledPackage::entry_points`] per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryRole {
    /// The command-line executable (`php.exe`, `redis-cli.exe`).
    Cli,
    /// The FastCGI executable Apache runs (`php-cgi.exe`).
    FastCgi,
    /// The directory holding the command-line clients (`mysql.exe`, `mysqldump.exe`).
    ClientDir,
    /// A PHP archive run with PHP (`composer.phar`).
    Phar,
    /// A module Apache loads (`mod_fcgid.so`).
    ApacheModule,
    /// A PHP extension DLL loaded through php.ini (`php_xdebug.dll`); its name is
    /// [`InstalledPackage::php_extension`].
    PhpExtension,
}

/// A package present on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub id: PackageId,
    pub install_dir: PathBuf,
    /// Visual C++ runtime the build needs, as its catalog entry said at install time (`None`:
    /// none needed, or unknown for packages recorded before this was tracked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vc_runtime: Option<crate::vc_runtime::VcRuntime>,
    /// Resolved files by role (see [`EntryRole`]); filled by `InstalledPackageRegistry::list`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entry_points: BTreeMap<EntryRole, PathBuf>,
    /// For add-on extensions: the extension name php.ini loads (`xdebug`, `redis`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub php_extension: Option<crate::php::ExtensionName>,
}

impl InstalledPackage {
    /// A package whose entry points are not resolved yet (before it is recorded).
    pub fn new(id: PackageId, install_dir: PathBuf, vc_runtime: Option<crate::vc_runtime::VcRuntime>) -> Self {
        Self { id, install_dir, vc_runtime, entry_points: BTreeMap::new(), php_extension: None }
    }

    /// The file (or directory) playing `role`, if this package has one.
    pub fn entry(&self, role: EntryRole) -> Option<&Path> {
        self.entry_points.get(&role).map(PathBuf::as_path)
    }

    /// The runtime this build needs: the recorded one, else the one of the official PHP builds of
    /// its minor (PHP is the one kind whose toolset follows from the version).
    pub fn required_vc_runtime(&self) -> Option<crate::vc_runtime::VcRuntime> {
        self.vc_runtime.or_else(|| match self.id.kind == PackageKind::PHP {
            true => crate::php::PhpMinor::try_from(self.id.series()).ok().map(crate::vc_runtime::VcRuntime::for_php),
            false => None,
        })
    }
}

/// Outcome of comparing a computed digest with the expected one.
pub fn verify_checksum(file: &str, expected: &Sha256Digest, actual: &Sha256Digest) -> Result<(), crate::error::PortError> {
    if expected == actual { Ok(()) } else { Err(crate::error::PortError::ChecksumMismatch { file: file.to_string(), expected: expected.to_string(), actual: actual.to_string() }) }
}

/// The newest entry of `kind` within `series` (optionally for a given PHP minor). Used for
/// one-click patch updates and for "install latest".
pub fn latest_in_series<'a>(entries: &'a [CatalogEntry], kind: &PackageKind, series: VersionSeries, php: Option<PhpMinor>) -> Option<&'a CatalogEntry> {
    entries.iter().filter(|e| &e.id.kind == kind && e.id.series() == series && e.id.php == php).max_by(|a, b| a.id.version.cmp(&b.id.version))
}

/// The newest entry of `kind` overall (optionally for a given PHP minor).
pub fn latest<'a>(entries: &'a [CatalogEntry], kind: &PackageKind, php: Option<PhpMinor>) -> Option<&'a CatalogEntry> {
    entries.iter().filter(|e| &e.id.kind == kind && e.id.php == php).max_by(|a, b| a.id.version.cmp(&b.id.version))
}

/// Merges entries from several sources: on duplicate ids the *first* source wins
/// (the composition root orders sources, e.g. live before frozen). Output is sorted by id.
pub fn merge_catalogs(sources: impl IntoIterator<Item = Vec<CatalogEntry>>) -> Vec<CatalogEntry> {
    let mut seen = std::collections::BTreeMap::new();
    for list in sources {
        for entry in list {
            seen.entry(entry.id.clone()).or_insert(entry);
        }
    }
    seen.into_values().collect()
}

/// The newest installed package of `kind` in `series`, e.g. the PHP 8.3 build a project uses.
pub fn newest_installed<'a>(installed: &'a [InstalledPackage], kind: &PackageKind, series: VersionSeries, php: Option<PhpMinor>) -> Option<&'a InstalledPackage> {
    installed.iter().filter(|p| &p.id.kind == kind && p.id.series() == series && p.id.php == php).max_by(|a, b| a.id.version.cmp(&b.id.version))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: PackageKind, version: &str, php: Option<&str>) -> CatalogEntry {
        CatalogEntry {
            id: PackageId { kind, version: PackageVersion::parse(version).unwrap(), php: php.map(|p| PhpMinor::parse(p).unwrap()) },
            url: DownloadUrl::new(format!("https://example.invalid/{version}.zip")).unwrap(),
            sha256: None,
            size_bytes: None,
            vc_runtime: None,
            channel: CatalogChannel::Frozen,
            end_of_life: false,
            payload: PackagePayload::Zip { strip_prefix: None },
        }
    }

    #[test]
    fn kind_validation() {
        assert_eq!(PackageKind::new("php").unwrap(), PackageKind::PHP);
        assert!(PackageKind::new("PHP").is_err());
        assert!(PackageKind::new("").is_err());
        assert!(PackageKind::new("a b").is_err());
    }

    #[test]
    fn package_key_is_filesystem_safe() {
        let id = PackageId::for_php(PackageKind::XDEBUG, PackageVersion::parse("3.3.2").unwrap(), PhpMinor::parse("8.3").unwrap());
        assert_eq!(id.key(), "xdebug-3.3.2-php8.3");
    }

    #[test]
    fn sha256_hex_roundtrip() {
        let hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let d = Sha256Digest::parse_hex(hex).unwrap();
        assert_eq!(d.to_string(), hex);
        assert_eq!(Sha256Digest::parse_hex(&hex.to_uppercase()).unwrap(), d);
        assert!(Sha256Digest::parse_hex("abc").is_err());
        assert!(verify_checksum("f", &d, &d).is_ok());
        assert!(verify_checksum("f", &d, &Sha256Digest::from_bytes([0; 32])).is_err());
    }

    #[test]
    fn urls_must_be_https() {
        assert!(DownloadUrl::new("http://windows.php.net/x.zip").is_err());
        let u = DownloadUrl::new("https://windows.php.net/downloads/releases/php-8.3.14-nts-Win32-vs16-x64.zip?x=1").unwrap();
        assert_eq!(u.file_name(), "php-8.3.14-nts-Win32-vs16-x64.zip");
    }

    #[test]
    fn picks_latest_patch_in_series() {
        let entries =
            vec![entry(PackageKind::PHP, "8.3.9", None), entry(PackageKind::PHP, "8.3.14", None), entry(PackageKind::PHP, "8.4.1", None), entry(PackageKind::XDEBUG, "3.3.2", Some("8.3"))];
        let series = VersionSeries { major: 8, minor: 3 };
        assert_eq!(latest_in_series(&entries, &PackageKind::PHP, series, None).unwrap().id.version.as_str(), "8.3.14");
        assert_eq!(latest(&entries, &PackageKind::PHP, None).unwrap().id.version.as_str(), "8.4.1");
        let php83 = PhpMinor::parse("8.3").unwrap();
        assert!(latest(&entries, &PackageKind::XDEBUG, Some(php83)).is_some());
        assert!(latest(&entries, &PackageKind::XDEBUG, None).is_none());
    }

    #[test]
    fn merge_keeps_first_source_on_duplicates() {
        let mut live = entry(PackageKind::PHP, "8.3.14", None);
        live.channel = CatalogChannel::Live;
        let frozen = entry(PackageKind::PHP, "8.3.14", None);
        let merged = merge_catalogs([vec![live], vec![frozen, entry(PackageKind::PHP, "5.6.40", None)]]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged.iter().find(|e| e.id.version.as_str() == "8.3.14").unwrap().channel, CatalogChannel::Live);
    }
}
