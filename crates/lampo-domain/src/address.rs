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
use crate::tcp_port::Port;

/// First port of the range suggested for dedicated loopback listeners.
pub const LOOPBACK_PORT_RANGE_START: Port = Port::of(8100);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "SiteAddressRepr", into = "SiteAddressRepr")]
pub enum SiteAddress {
    /// `shop.test` — needs an entry in the Lampo hosts-file block.
    Hostname(Hostname),
    /// `shop.localhost` — no hosts entry needed. Served as a name-based vhost on the main port.
    Localhost(Hostname),
    /// `127.0.0.1:<port>` — a dedicated Apache listener for this site only.
    LoopbackPort(Port),
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

    /// A dedicated listener: never on a privileged port.
    pub fn loopback_port(port: Port) -> Result<Self, DomainError> {
        if !port.is_unprivileged() {
            return Err(DomainError::InvalidPort(u32::from(port.get())));
        }
        Ok(Self::LoopbackPort(port))
    }

    /// Default addresses of a new project: `<slug>.<tld>` plus `<slug>.localhost`, so the site is
    /// reachable immediately (via `.localhost`) even before the hosts file is updated.
    pub fn defaults_for(slug: &str, tld: &Tld) -> Result<Vec<Self>, DomainError> {
        Ok(vec![Self::hostname(Hostname::for_project(slug, tld)?)?, Self::localhost(slug)?])
    }

    /// Parses the displayed form ([`fmt::Display`]): `shop.test` (hosts file), `shop.localhost`,
    /// `127.0.0.1:8101`.
    pub fn parse(text: &str) -> Result<Self, DomainError> {
        let text = text.trim();
        if let Some(port) = text.strip_prefix("127.0.0.1:") {
            let port = port.parse::<u16>().map_err(|_| DomainError::InvalidAddresses(format!("`{text}`: the port must be a number from 1024 to 65535")))?;
            return Self::loopback_port(Port::try_from(port)?);
        }
        let host = Hostname::new(text)?;
        if host.is_localhost() { Ok(Self::Localhost(host)) } else { Self::hostname(host) }
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

    pub fn loopback(&self) -> Option<Port> {
        match self {
            Self::LoopbackPort(p) => Some(*p),
            _ => None,
        }
    }

    /// Browser URL. `apache_port` is the shared name-based listener.
    pub fn url(&self, apache_port: Port, tls: TlsMode) -> String {
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

/// Which default addresses a new project gets when none are given explicitly. Stored as the
/// global default (`Settings::default_addressing`, kebab-case in the file).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Addressing {
    /// `<id>.<tld>` (hosts file) + `<id>.localhost`.
    Standard,
    /// `<id>.localhost` + a dedicated loopback port: never needs elevation.
    #[default]
    NoElevation,
    /// Only `<id>.<tld>` (hosts file).
    HostsOnly,
}

impl Addressing {
    /// `true` when the addresses include a dedicated loopback port, which the caller allocates.
    pub fn needs_loopback_port(self) -> bool {
        self == Self::NoElevation
    }

    /// The addresses of project `slug`. `loopback_port` is used (and required) only when
    /// [`Addressing::needs_loopback_port`].
    pub fn addresses_for(self, slug: &str, tld: &Tld, loopback_port: Option<Port>) -> Result<Vec<SiteAddress>, DomainError> {
        match (self, loopback_port) {
            (Self::Standard, _) => SiteAddress::defaults_for(slug, tld),
            (Self::HostsOnly, _) => Ok(vec![SiteAddress::hostname(Hostname::for_project(slug, tld)?)?]),
            (Self::NoElevation, Some(port)) => Ok(vec![SiteAddress::localhost(slug)?, SiteAddress::loopback_port(port)?]),
            (Self::NoElevation, None) => Err(DomainError::InvalidAddresses("no-elevation addressing needs a dedicated loopback port".into())),
        }
    }
}

/// The stored form: `standard`, `no-elevation`, `hosts-only`.
impl fmt::Display for Addressing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Standard => "standard",
            Self::NoElevation => "no-elevation",
            Self::HostsOnly => "hosts-only",
        })
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
            SiteAddressRepr::LoopbackPort { port } => Self::loopback_port(Port::try_from(port)?),
        }
    }
}

impl From<SiteAddress> for SiteAddressRepr {
    fn from(a: SiteAddress) -> Self {
        match a {
            SiteAddress::Hostname(h) => Self::Hostname { host: h.into() },
            SiteAddress::Localhost(h) => Self::Localhost { host: h.into() },
            SiteAddress::LoopbackPort(port) => Self::LoopbackPort { port: port.get() },
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
    fn addressing_modes_give_their_addresses() {
        let tld = Tld::default();
        let names = |a: Addressing, port: Option<Port>| a.addresses_for("shop", &tld, port).unwrap().iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(names(Addressing::Standard, None), ["shop.test", "shop.localhost"]);
        assert_eq!(names(Addressing::HostsOnly, None), ["shop.test"]);
        assert_eq!(names(Addressing::NoElevation, Some(Port::of(8101))), ["shop.localhost", "127.0.0.1:8101"]);
        assert!(Addressing::NoElevation.addresses_for("shop", &tld, None).is_err());
        assert_eq!(Addressing::default(), Addressing::NoElevation);
        assert!(Addressing::NoElevation.needs_loopback_port());
        assert!(!Addressing::HostsOnly.needs_loopback_port() && !Addressing::Standard.needs_loopback_port());
    }

    #[test]
    fn addressing_is_kebab_case_on_the_wire() {
        assert_eq!(serde_json::to_string(&Addressing::HostsOnly).unwrap(), r#""hosts-only""#);
        assert_eq!(serde_json::from_str::<Addressing>(r#""no-elevation""#).unwrap(), Addressing::NoElevation);
        for a in [Addressing::Standard, Addressing::NoElevation, Addressing::HostsOnly] {
            assert_eq!(serde_json::to_string(&a).unwrap(), format!("\"{a}\""), "Display matches the stored form");
        }
    }

    #[test]
    fn variants_are_kept_apart() {
        assert!(SiteAddress::hostname(Hostname::new("shop.localhost").unwrap()).is_err());
        assert!(SiteAddress::localhost("feature-x.shop").is_ok());
        assert!(SiteAddress::loopback_port(Port::of(80)).is_err());
    }

    #[test]
    fn urls() {
        let host = SiteAddress::hostname(Hostname::new("shop.test").unwrap()).unwrap();
        assert_eq!(host.url(Port::of(80), TlsMode::Disabled), "http://shop.test");
        assert_eq!(host.url(Port::of(8080), TlsMode::Disabled), "http://shop.test:8080");
        assert_eq!(SiteAddress::LoopbackPort(Port::of(8101)).url(Port::of(80), TlsMode::Disabled), "http://127.0.0.1:8101");
    }

    #[test]
    fn serde_validates() {
        let json = serde_json::to_string(&SiteAddress::LoopbackPort(Port::of(8101))).unwrap();
        assert_eq!(json, r#"{"kind":"loopback-port","port":8101}"#);
        assert!(serde_json::from_str::<SiteAddress>(r#"{"kind":"localhost","host":"shop.test"}"#).is_err());
        assert!(serde_json::from_str::<SiteAddress>(r#"{"kind":"hostname","host":"shop.localhost"}"#).is_err());
    }

    #[test]
    fn parse_round_trips_the_displayed_form() {
        for text in ["shop.test", "api.shop.localhost", "127.0.0.1:8101"] {
            assert_eq!(SiteAddress::parse(text).unwrap().to_string(), text);
        }
        assert!(SiteAddress::parse(" Shop.Test ").unwrap().needs_hosts_entry());
        assert!(matches!(SiteAddress::parse("shop.localhost").unwrap(), SiteAddress::Localhost(_)));
        assert_eq!(SiteAddress::parse("127.0.0.1:8101").unwrap().loopback(), Some(Port::of(8101)));
        for bad in ["", "shop", "127.0.0.1:80", "127.0.0.1:x", "127.0.0.1:70000", "127.0.0.1", "http://shop.test"] {
            assert!(SiteAddress::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn duplicates_rejected() {
        let a = SiteAddress::LoopbackPort(Port::of(8101));
        assert!(validate_addresses(&[a.clone(), a]).is_err());
        assert!(validate_addresses(&[]).is_err());
    }
}
