//! How a site is reached. Besides a classic hostname written to the hosts file (UAC prompt when
//! the block changes), a site can be addressed without touching the hosts file at all:
//!
//! * `*.localhost` (RFC 6761) — browsers and curl resolve it to loopback by themselves;
//! * a dedicated loopback port — `http://127.0.0.1:<port>`, no name resolution involved.
//!
//! Both need no elevation, which matters for coding agents and keeps future per-worktree
//! environments (e.g. `feature-x.shop.localhost`) possible without UAC.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::hostname::{Hostname, Tld};
use crate::project::TlsMode;

/// First port of the range suggested for dedicated loopback listeners.
pub const LOOPBACK_PORT_RANGE_START: u16 = 8100;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "SiteAddressRepr", into = "SiteAddressRepr")]
pub enum SiteAddress {
    /// `shop.test` — needs an entry in the Lampo hosts-file block.
    Hostname(Hostname),
    /// `shop.localhost` — no hosts entry needed. Served as a name-based vhost on the main port.
    Localhost(Hostname),
    /// `127.0.0.1:<port>` — a dedicated Apache listener for this site only.
    LoopbackPort(u16),
}

impl SiteAddress {
    /// A hosts-file hostname. `.localhost` names must use [`SiteAddress::localhost`].
    pub fn hostname(host: Hostname) -> Result<Self, DomainError> {
        if host.is_localhost() {
            return Err(DomainError::InvalidHostname { value: host.to_string(), reason: "`.localhost` names need no hosts entry; use a localhost address".into() });
        }
        Ok(Self::Hostname(host))
    }

    /// `<label>.localhost`; `label` may contain dots (`feature-x.shop`).
    pub fn localhost(label: &str) -> Result<Self, DomainError> {
        Ok(Self::Localhost(Hostname::new(format!("{label}.localhost"))?))
    }

    pub fn loopback_port(port: u16) -> Result<Self, DomainError> {
        if port < 1024 {
            return Err(DomainError::InvalidPort(port as u32));
        }
        Ok(Self::LoopbackPort(port))
    }

    /// Default addresses of a new project: `<slug>.<tld>` plus `<slug>.localhost`, so the site is
    /// reachable immediately (via `.localhost`) even before the hosts file is updated.
    pub fn defaults_for(slug: &str, tld: &Tld) -> Result<Vec<Self>, DomainError> {
        Ok(vec![Self::hostname(Hostname::for_project(slug, tld)?)?, Self::localhost(slug)?])
    }

    pub fn needs_hosts_entry(&self) -> bool {
        matches!(self, Self::Hostname(_))
    }

    /// The DNS name, if this address is name-based.
    pub fn host(&self) -> Option<&Hostname> {
        match self {
            Self::Hostname(h) | Self::Localhost(h) => Some(h),
            Self::LoopbackPort(_) => None,
        }
    }

    pub fn loopback(&self) -> Option<u16> {
        match self {
            Self::LoopbackPort(p) => Some(*p),
            _ => None,
        }
    }

    /// Browser URL. `apache_port` is the shared name-based listener.
    pub fn url(&self, apache_port: u16, tls: TlsMode) -> String {
        let scheme = if tls == TlsMode::LocalCa { "https" } else { "http" };
        let default_port = if tls == TlsMode::LocalCa { 443 } else { 80 };
        match self {
            Self::Hostname(h) | Self::Localhost(h) if apache_port == default_port => format!("{scheme}://{h}"),
            Self::Hostname(h) | Self::Localhost(h) => format!("{scheme}://{h}:{apache_port}"),
            Self::LoopbackPort(p) => format!("{scheme}://127.0.0.1:{p}"),
        }
    }
}

impl fmt::Display for SiteAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hostname(h) | Self::Localhost(h) => write!(f, "{h}"),
            Self::LoopbackPort(p) => write!(f, "127.0.0.1:{p}"),
        }
    }
}

/// Validates a project's address list: non-empty, no duplicates.
pub fn validate_addresses(addresses: &[SiteAddress]) -> Result<(), DomainError> {
    if addresses.is_empty() {
        return Err(DomainError::InvalidAddresses("a project needs at least one address".into()));
    }
    for (i, a) in addresses.iter().enumerate() {
        if addresses[..i].contains(a) {
            return Err(DomainError::InvalidAddresses(format!("`{a}` is listed twice")));
        }
    }
    if addresses.iter().filter(|a| a.loopback().is_some()).count() > 1 {
        return Err(DomainError::InvalidAddresses("a project can have at most one dedicated loopback port".into()));
    }
    Ok(())
}

/// Serialized form: `{ "kind": "hostname", "host": "shop.test" }`, `{ "kind": "loopback-port", "port": 8101 }`.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum SiteAddressRepr {
    Hostname { host: String },
    Localhost { host: String },
    LoopbackPort { port: u16 },
}

impl TryFrom<SiteAddressRepr> for SiteAddress {
    type Error = DomainError;
    fn try_from(r: SiteAddressRepr) -> Result<Self, Self::Error> {
        match r {
            SiteAddressRepr::Hostname { host } => Self::hostname(Hostname::new(host)?),
            SiteAddressRepr::Localhost { host } => {
                let h = Hostname::new(&host)?;
                if !h.is_localhost() {
                    return Err(DomainError::InvalidHostname { value: host, reason: "must end with `.localhost`".into() });
                }
                Ok(Self::Localhost(h))
            }
            SiteAddressRepr::LoopbackPort { port } => Self::loopback_port(port),
        }
    }
}

impl From<SiteAddress> for SiteAddressRepr {
    fn from(a: SiteAddress) -> Self {
        match a {
            SiteAddress::Hostname(h) => Self::Hostname { host: h.into() },
            SiteAddress::Localhost(h) => Self::Localhost { host: h.into() },
            SiteAddress::LoopbackPort(port) => Self::LoopbackPort { port },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_include_a_uac_free_address() {
        let a = SiteAddress::defaults_for("shop", &Tld::default()).unwrap();
        assert_eq!(a.iter().map(|x| x.to_string()).collect::<Vec<_>>(), ["shop.test", "shop.localhost"]);
        assert!(a[0].needs_hosts_entry());
        assert!(!a[1].needs_hosts_entry());
    }

    #[test]
    fn variants_are_kept_apart() {
        assert!(SiteAddress::hostname(Hostname::new("shop.localhost").unwrap()).is_err());
        assert!(SiteAddress::localhost("feature-x.shop").is_ok());
        assert!(SiteAddress::loopback_port(80).is_err());
    }

    #[test]
    fn urls() {
        let host = SiteAddress::hostname(Hostname::new("shop.test").unwrap()).unwrap();
        assert_eq!(host.url(80, TlsMode::Disabled), "http://shop.test");
        assert_eq!(host.url(8080, TlsMode::Disabled), "http://shop.test:8080");
        assert_eq!(SiteAddress::LoopbackPort(8101).url(80, TlsMode::Disabled), "http://127.0.0.1:8101");
    }

    #[test]
    fn serde_validates() {
        let json = serde_json::to_string(&SiteAddress::LoopbackPort(8101)).unwrap();
        assert_eq!(json, r#"{"kind":"loopback-port","port":8101}"#);
        assert!(serde_json::from_str::<SiteAddress>(r#"{"kind":"localhost","host":"shop.test"}"#).is_err());
        assert!(serde_json::from_str::<SiteAddress>(r#"{"kind":"hostname","host":"shop.localhost"}"#).is_err());
    }

    #[test]
    fn duplicates_rejected() {
        let a = SiteAddress::LoopbackPort(8101);
        assert!(validate_addresses(&[a.clone(), a]).is_err());
        assert!(validate_addresses(&[]).is_err());
    }
}
