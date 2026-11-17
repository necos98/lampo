//! PHP-specific value objects: versions, extensions, php.ini overrides, Composer channel.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::version::{PackageVersion, VersionSeries};

mod extension_alias;

pub use extension_alias::extensions_for_php;

/// A PHP *minor* version (`8.3`). Projects bind to a minor, never to a patch,
/// so a patch update is a one-click operation that does not touch any project.
///
/// Supported: 5.6, 7.0–7.4, 8.x and any later major.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PhpMinor {
    major: u8,
    minor: u8,
}

impl PhpMinor {
    /// The oldest PHP supported by Lampo.
    pub const OLDEST: PhpMinor = PhpMinor { major: 5, minor: 6 };
    /// Fallback "latest" used only when neither the catalog nor installed packages can tell
    /// (e.g. offline first run). Keep it updated when cutting a release.
    pub const LATEST_KNOWN: PhpMinor = PhpMinor { major: 8, minor: 4 };
    /// The oldest PHP the Laravel Installer runs on (`laravel/installer` requires `^8.2`).
    pub const LARAVEL_INSTALLER_MIN: PhpMinor = PhpMinor { major: 8, minor: 2 };
    /// The oldest PHP whose `mysqlnd` speaks MySQL 8's `caching_sha2_password`
    /// (see [`crate::mysql::auth`]).
    pub const MYSQL_CACHING_SHA2_MIN: PhpMinor = PhpMinor { major: 7, minor: 4 };

    pub fn new(major: u8, minor: u8) -> Result<Self, DomainError> {
        let ok = match major {
            0..=4 => false,
            5 => minor == 6,
            7 => minor <= 4,
            6 => false, // PHP 6 never shipped.
            _ => minor <= 99,
        };
        if ok { Ok(Self { major, minor }) } else { Err(DomainError::InvalidPhpVersion(format!("{major}.{minor}"))) }
    }

    pub fn parse(input: &str) -> Result<Self, DomainError> {
        let err = || DomainError::InvalidPhpVersion(input.to_string());
        let (ma, mi) = input.trim().split_once('.').ok_or_else(err)?;
        let num = |s: &str| if !s.is_empty() && s.len() <= 2 && s.bytes().all(|b| b.is_ascii_digit()) { s.parse::<u8>().ok() } else { None };
        let (Some(major), Some(minor)) = (num(ma), num(mi)) else { return Err(err()) };
        Self::new(major, minor).map_err(|_| err())
    }

    pub fn major(self) -> u8 {
        self.major
    }

    pub fn minor(self) -> u8 {
        self.minor
    }

    pub fn as_series(self) -> VersionSeries {
        VersionSeries { major: self.major as u32, minor: self.minor as u32 }
    }

    /// Whether `version` (a full PHP release such as `8.3.14`) belongs to this minor.
    pub fn contains(self, version: &PackageVersion) -> bool {
        version.series() == self.as_series()
    }

    /// Composer channel usable with this PHP: Composer 2.2 LTS for PHP < 7.2.
    pub fn composer_channel(self) -> ComposerChannel {
        ComposerChannel::for_php(self)
    }
}

impl fmt::Display for PhpMinor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for PhpMinor {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for PhpMinor {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

impl From<PhpMinor> for String {
    fn from(v: PhpMinor) -> Self {
        v.to_string()
    }
}

impl TryFrom<VersionSeries> for PhpMinor {
    type Error = DomainError;
    fn try_from(s: VersionSeries) -> Result<Self, Self::Error> {
        let err = || DomainError::InvalidPhpVersion(s.to_string());
        let major = u8::try_from(s.major).map_err(|_| err())?;
        let minor = u8::try_from(s.minor).map_err(|_| err())?;
        Self::new(major, minor)
    }
}

/// Extracts the *lowest* PHP minor allowed by a Composer constraint (`require.php` in
/// `composer.json`), e.g. `^8.2` → 8.2, `>=7.4 <8.3` → 7.4, `^7.3|^8.0` → 7.3, `8.1.*` → 8.1.
///
/// Returns `None` when no version can be read or the lowest bound is older than PHP 5.6
/// (in which case the caller should fall back to its default).
pub fn minimum_php_from_constraint(constraint: &str) -> Option<PhpMinor> {
    constraint
        .split('|')
        .filter(|alt| !alt.trim().is_empty())
        .filter_map(|alt| {
            // First version-looking token of the alternative, ignoring upper bounds (`<`, `<=`).
            alt.split([' ', ',']).filter(|t| !t.is_empty()).find_map(|token| {
                if token.starts_with('<') || token.starts_with("!=") {
                    return None;
                }
                let digits: String = token.trim_start_matches(['^', '~', '>', '=', 'v']).chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
                let mut parts = digits.split('.').filter(|p| !p.is_empty());
                let major: u8 = parts.next()?.parse().ok()?;
                let minor: u8 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
                PhpMinor::new(major, minor).ok().or_else(|| if major == 5 && minor < 6 { None } else { PhpMinor::new(major, 0).ok() })
            })
        })
        .min()
}

/// Which Composer line to run for a given PHP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComposerChannel {
    /// Composer 2.2 LTS — the last line supporting PHP < 7.2.5.
    Lts22,
    /// Latest stable Composer 2.x.
    Stable,
}

impl ComposerChannel {
    pub fn for_php(php: PhpMinor) -> Self {
        const FIRST_STABLE: PhpMinor = PhpMinor { major: 7, minor: 2 };
        if php < FIRST_STABLE { Self::Lts22 } else { Self::Stable }
    }
}

/// Name of a PHP extension as written in `php.ini` (`pdo_mysql`, `intl`, `xdebug`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExtensionName(String);

impl ExtensionName {
    pub fn new(name: impl AsRef<str>) -> Result<Self, DomainError> {
        // Accept the DLL file name too (`php_intl.dll` → `intl`).
        let mut n = name.as_ref().trim().to_ascii_lowercase();
        if let Some(s) = n.strip_suffix(".dll") {
            n = s.to_string();
        }
        if let Some(s) = n.strip_prefix("php_") {
            n = s.to_string();
        }
        let valid = !n.is_empty() && n.len() <= 64 && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') && n.as_bytes()[0].is_ascii_lowercase();
        if valid { Ok(Self(n)) } else { Err(DomainError::InvalidExtensionName(name.as_ref().to_string())) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExtensionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ExtensionName {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<ExtensionName> for String {
    fn from(v: ExtensionName) -> Self {
        v.0
    }
}

/// A php.ini directive name (`memory_limit`, `xdebug.mode`, `opcache.enable`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct IniKey(String);

impl IniKey {
    pub fn new(key: impl AsRef<str>) -> Result<Self, DomainError> {
        let k = key.as_ref().trim();
        let valid = !k.is_empty()
            && k.len() <= 128
            && k.as_bytes()[0].is_ascii_alphabetic()
            && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
            // Extensions are managed through the extension set, not raw ini lines.
            && !k.eq_ignore_ascii_case("extension")
            && !k.eq_ignore_ascii_case("zend_extension");
        if valid { Ok(Self(k.to_string())) } else { Err(DomainError::InvalidIniKey(key.as_ref().to_string())) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IniKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for IniKey {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<IniKey> for String {
    fn from(v: IniKey) -> Self {
        v.0
    }
}

/// A php.ini value. Single line, no NUL; quoting is the renderer's job.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IniValue(String);

impl IniValue {
    pub fn new(key: &IniKey, value: impl Into<String>) -> Result<Self, DomainError> {
        let v: String = value.into();
        if v.contains(['\n', '\r', '\0']) {
            return Err(DomainError::InvalidIniValue { key: key.to_string(), reason: "must be a single line".into() });
        }
        if v.len() > 4096 {
            return Err(DomainError::InvalidIniValue { key: key.to_string(), reason: "too long".into() });
        }
        Ok(Self(v))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Per-project php.ini overrides, applied on top of Lampo's base php.ini.
pub type PhpIniOverrides = BTreeMap<IniKey, IniValue>;

/// Builds a validated override map from raw pairs (e.g. coming from a DTO).
pub fn php_ini_overrides<I, K, V>(pairs: I) -> Result<PhpIniOverrides, DomainError>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: Into<String>,
{
    pairs
        .into_iter()
        .map(|(k, v)| {
            let key = IniKey::new(k)?;
            let value = IniValue::new(&key, v)?;
            Ok((key, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(s: &str) -> PhpMinor {
        PhpMinor::parse(s).unwrap()
    }

    #[test]
    fn parses_supported_minors() {
        assert_eq!(m("5.6").to_string(), "5.6");
        assert_eq!(m("7.4").to_string(), "7.4");
        assert_eq!(m(" 8.3 ").to_string(), "8.3");
        assert_eq!(m("8.10").minor(), 10);
    }

    #[test]
    fn rejects_unsupported_or_malformed_minors() {
        for bad in ["5.5", "5.6.1", "6.0", "7.5", "4.4", "8", "8.", ".3", "x.y", "8.3a", "300.1"] {
            assert!(PhpMinor::parse(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn minors_are_ordered() {
        assert!(m("5.6") < m("7.0"));
        assert!(m("7.4") < m("8.0"));
        assert!(m("8.2") < m("8.10"));
    }

    #[test]
    fn minor_contains_patch_versions() {
        let v = PackageVersion::parse("8.3.14").unwrap();
        assert!(m("8.3").contains(&v));
        assert!(!m("8.2").contains(&v));
    }

    #[test]
    fn composer_channel_switches_at_7_2() {
        assert_eq!(m("5.6").composer_channel(), ComposerChannel::Lts22);
        assert_eq!(m("7.1").composer_channel(), ComposerChannel::Lts22);
        assert_eq!(m("7.2").composer_channel(), ComposerChannel::Stable);
        assert_eq!(m("8.4").composer_channel(), ComposerChannel::Stable);
    }

    #[test]
    fn reads_lowest_minor_from_composer_constraints() {
        assert_eq!(minimum_php_from_constraint("^8.2"), Some(m("8.2")));
        assert_eq!(minimum_php_from_constraint(">=7.4 <8.3"), Some(m("7.4")));
        assert_eq!(minimum_php_from_constraint("^7.3|^8.0"), Some(m("7.3")));
        assert_eq!(minimum_php_from_constraint("^8.0 || ^7.2.5"), Some(m("7.2")));
        assert_eq!(minimum_php_from_constraint("~7.1.3"), Some(m("7.1")));
        assert_eq!(minimum_php_from_constraint("8.1.*"), Some(m("8.1")));
        assert_eq!(minimum_php_from_constraint(">=8"), Some(m("8.0")));
        assert_eq!(minimum_php_from_constraint("*"), None);
        assert_eq!(minimum_php_from_constraint(">=5.3.9"), None);
    }

    #[test]
    fn extension_names_are_normalised() {
        assert_eq!(ExtensionName::new("pdo_mysql").unwrap().as_str(), "pdo_mysql");
        assert_eq!(ExtensionName::new("php_intl.dll").unwrap().as_str(), "intl");
        assert_eq!(ExtensionName::new("Xdebug").unwrap().as_str(), "xdebug");
        assert!(ExtensionName::new("").is_err());
        assert!(ExtensionName::new("../evil").is_err());
        assert!(ExtensionName::new("1abc").is_err());
    }

    #[test]
    fn ini_overrides_are_validated() {
        let ok = php_ini_overrides([("memory_limit", "512M"), ("xdebug.mode", "debug")]).unwrap();
        assert_eq!(ok.len(), 2);
        assert!(php_ini_overrides([("extension", "foo")]).is_err());
        assert!(php_ini_overrides([("memory_limit", "1\nextension=evil")]).is_err());
        assert!(php_ini_overrides([("bad key", "1")]).is_err());
    }

    #[test]
    fn php_minor_serializes_as_string() {
        assert_eq!(serde_json::to_string(&m("8.3")).unwrap(), "\"8.3\"");
        assert!(serde_json::from_str::<PhpMinor>("\"5.5\"").is_err());
    }
}
