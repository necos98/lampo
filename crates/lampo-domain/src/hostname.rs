//! Local hostnames (`shop.test`) used for virtual hosts and the hosts-file block.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

fn valid_label(label: &str) -> bool {
    !label.is_empty() && label.len() <= 63 && label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !label.starts_with('-') && !label.ends_with('-')
}

/// Top-level domain used for generated hostnames (default `test`, reserved by RFC 2606).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Tld(String);

impl Tld {
    pub fn new(tld: impl AsRef<str>) -> Result<Self, DomainError> {
        let t = tld.as_ref().trim().trim_start_matches('.').to_ascii_lowercase();
        if !valid_label(&t) || t.bytes().all(|b| b.is_ascii_digit()) {
            return Err(DomainError::InvalidHostname { value: tld.as_ref().to_string(), reason: "not a valid top-level domain".into() });
        }
        // `.dev` and `.app` are HSTS-preloaded: browsers force HTTPS and sites break.
        if matches!(t.as_str(), "dev" | "app" | "com" | "net" | "org" | "localhost") {
            return Err(DomainError::InvalidHostname { value: t, reason: "this top-level domain cannot be used for local sites".into() });
        }
        Ok(Self(t))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Tld {
    fn default() -> Self {
        Self("test".into())
    }
}

impl fmt::Display for Tld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Tld {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<Tld> for String {
    fn from(v: Tld) -> Self {
        v.0
    }
}

/// A lowercase DNS hostname with at least two labels, e.g. `shop.test` or `api.shop.test`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hostname(String);

impl Hostname {
    pub fn new(value: impl AsRef<str>) -> Result<Self, DomainError> {
        let raw = value.as_ref();
        let h = raw.trim().trim_end_matches('.').to_ascii_lowercase();
        let fail = |reason: &str| DomainError::InvalidHostname { value: raw.to_string(), reason: reason.to_string() };
        if h.is_empty() || h.len() > 253 {
            return Err(fail("length must be 1-253 characters"));
        }
        let labels: Vec<&str> = h.split('.').collect();
        if labels.len() < 2 {
            return Err(fail("must contain a dot, e.g. `shop.test`"));
        }
        if !labels.iter().all(|l| valid_label(l)) {
            return Err(fail("use letters, digits and dashes between dots"));
        }
        if labels.last().is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit())) {
            return Err(fail("cannot be an IP address"));
        }
        Ok(Self(h))
    }

    /// `true` for `*.localhost` names (RFC 6761), which need no hosts-file entry.
    pub fn is_localhost(&self) -> bool {
        self.0.ends_with(".localhost")
    }

    /// The default hostname for a project: `<slug>.<tld>`.
    pub fn for_project(slug: &str, tld: &Tld) -> Result<Self, DomainError> {
        Self::new(format!("{slug}.{tld}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Hostname {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Hostname {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<Hostname> for String {
    fn from(v: Hostname) -> Self {
        v.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_and_normalises_hostnames() {
        assert_eq!(Hostname::new("Shop.Test").unwrap().as_str(), "shop.test");
        assert_eq!(Hostname::new("api.my-shop.test.").unwrap().as_str(), "api.my-shop.test");
        assert_eq!(Hostname::for_project("blog", &Tld::default()).unwrap().as_str(), "blog.test");
    }

    #[test]
    fn rejects_invalid_hostnames() {
        for bad in ["", "shop", "-shop.test", "shop-.test", "sh op.test", "shop..test", "127.0.0.1", "shöp.test"] {
            assert!(Hostname::new(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn localhost_names() {
        assert!(Hostname::new("shop.localhost").unwrap().is_localhost());
        assert!(!Hostname::new("localhost.test").unwrap().is_localhost());
    }

    #[test]
    fn tld_rules() {
        assert_eq!(Tld::new(".Local").unwrap().as_str(), "local");
        assert!(Tld::new("dev").is_err());
        assert!(Tld::new("com").is_err());
        assert!(Tld::new("a.b").is_err());
    }
}
