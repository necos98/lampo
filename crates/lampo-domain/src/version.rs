//! Generic dotted version used by every downloadable package
//! (Apache `2.4.62`, MySQL `8.4.3`, Redis for Windows `5.0.14.1`, Composer `2.2.24`, Xdebug `3.3.2`…).

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// A version made of 1–4 numeric components and an optional build suffix
/// (`2.4.62-240904` → components `[2, 4, 62]`, suffix `240904`).
///
/// Ordering compares the numeric components first (missing components count as 0),
/// then the suffix lexicographically. The original text is preserved for display.
#[derive(Debug, Clone, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PackageVersion {
    components: Vec<u32>,
    suffix: Option<String>,
    raw: String,
}

/// The `<major>.<minor>` series a version belongs to. Patch updates stay in the same series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct VersionSeries {
    pub major: u32,
    pub minor: u32,
}

impl fmt::Display for VersionSeries {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl VersionSeries {
    /// Exactly `<major>.<minor>` (`8.4`); anything else is `InvalidVersion`.
    pub fn parse(input: &str) -> Result<Self, DomainError> {
        let err = || DomainError::InvalidVersion(input.to_string());
        let (a, b) = input.trim().split_once('.').ok_or_else(err)?;
        let num = |s: &str| if !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()) { s.parse::<u32>().map_err(|_| err()) } else { Err(err()) };
        Ok(Self { major: num(a)?, minor: num(b)? })
    }
}

/// Serde helper storing an optional series as text (`"8.4"`) in config files.
pub mod series_text {
    use super::VersionSeries;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<VersionSeries>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(series) => s.serialize_str(&series.to_string()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<VersionSeries>, D::Error> {
        Option::<String>::deserialize(d)?.map(|t| VersionSeries::parse(&t).map_err(serde::de::Error::custom)).transpose()
    }
}

impl PackageVersion {
    pub fn parse(input: &str) -> Result<Self, DomainError> {
        let raw = input.trim();
        let err = || DomainError::InvalidVersion(input.to_string());
        if raw.is_empty() {
            return Err(err());
        }
        let (numeric, suffix) = match raw.find(['-', '+']) {
            Some(i) => (&raw[..i], Some(&raw[i + 1..])),
            None => (raw, None),
        };
        let components = numeric
            .split('.')
            .map(|p| if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) { Err(err()) } else { p.parse::<u32>().map_err(|_| err()) })
            .collect::<Result<Vec<_>, _>>()?;
        if components.is_empty() || components.len() > 4 {
            return Err(err());
        }
        let suffix = match suffix {
            Some(s) if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') => return Err(err()),
            Some(s) => Some(s.to_string()),
            None => None,
        };
        Ok(Self { components, suffix, raw: raw.to_string() })
    }

    pub fn components(&self) -> &[u32] {
        &self.components
    }

    pub fn major(&self) -> u32 {
        self.component(0)
    }

    pub fn minor(&self) -> u32 {
        self.component(1)
    }

    pub fn patch(&self) -> u32 {
        self.component(2)
    }

    pub fn suffix(&self) -> Option<&str> {
        self.suffix.as_deref()
    }

    pub fn series(&self) -> VersionSeries {
        VersionSeries { major: self.major(), minor: self.minor() }
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    fn component(&self, i: usize) -> u32 {
        self.components.get(i).copied().unwrap_or(0)
    }
}

impl PartialEq for PackageVersion {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl std::hash::Hash for PackageVersion {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Consistent with `Eq`: trailing zero components do not matter.
        let mut c = self.components.clone();
        while c.last() == Some(&0) {
            c.pop();
        }
        c.hash(state);
        self.suffix.hash(state);
    }
}

impl Ord for PackageVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let len = self.components.len().max(other.components.len());
        for i in 0..len {
            match self.component(i).cmp(&other.component(i)) {
                Ordering::Equal => continue,
                o => return o,
            }
        }
        self.suffix.cmp(&other.suffix)
    }
}

impl PartialOrd for PackageVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for PackageVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl FromStr for PackageVersion {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for PackageVersion {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

impl From<PackageVersion> for String {
    fn from(v: PackageVersion) -> Self {
        v.raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> PackageVersion {
        PackageVersion::parse(s).unwrap()
    }

    #[test]
    fn parses_common_versions() {
        assert_eq!(v("2.4.62").components(), &[2, 4, 62]);
        assert_eq!(v("5.0.14.1").components(), &[5, 0, 14, 1]);
        assert_eq!(v("2.4.62-240904").suffix(), Some("240904"));
        assert_eq!(v("8.4.3").series(), VersionSeries { major: 8, minor: 4 });
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "x", "1..2", "1.2.3.4.5", "1.2-", "-1", "1.a"] {
            assert!(PackageVersion::parse(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn orders_numerically() {
        assert!(v("8.3.10") > v("8.3.9"));
        assert!(v("2.4") == v("2.4.0"));
        assert!(v("5.0.14.1") > v("5.0.14"));
        let mut list = [v("8.4.1"), v("5.7.44"), v("8.0.40")];
        list.sort();
        assert_eq!(list.iter().map(|x| x.to_string()).collect::<Vec<_>>(), ["5.7.44", "8.0.40", "8.4.1"]);
    }

    #[test]
    fn serde_roundtrip_keeps_original_text() {
        let json = serde_json::to_string(&v("2.4.62-240904")).unwrap();
        assert_eq!(json, "\"2.4.62-240904\"");
        let back: PackageVersion = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_str(), "2.4.62-240904");
    }
}
