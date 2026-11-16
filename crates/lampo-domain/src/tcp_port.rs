//! TCP ports as value objects: a [`Port`] is never 0, a [`PortRange`] is never empty and never
//! reaches below the privileged ports. Both are stored on disk as plain integers
//! (`port = 8101`, `{ start = 8100, end = 8199 }`) and validated when read.

use std::fmt;
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// First port a user-assigned range may start from: below it are the privileged ports.
pub const FIRST_UNPRIVILEGED_PORT: u16 = 1024;

/// A TCP port, 1-65535.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct Port(NonZeroU16);

impl Port {
    /// `None` for 0.
    pub const fn new(port: u16) -> Option<Self> {
        match NonZeroU16::new(port) {
            Some(port) => Some(Self(port)),
            None => None,
        }
    }

    /// A port known at compile time (defaults, tests). Fails to compile for 0 in a `const`.
    pub const fn of(port: u16) -> Self {
        match Self::new(port) {
            Some(port) => port,
            None => panic!("a TCP port is never 0"),
        }
    }

    pub const fn get(self) -> u16 {
        self.0.get()
    }

    /// Not a privileged port (1024 and up): what a user-assigned loopback listener needs.
    pub const fn is_unprivileged(self) -> bool {
        self.get() >= FIRST_UNPRIVILEGED_PORT
    }

    /// This port and every one above it, up to 65535.
    pub fn upward(self) -> impl Iterator<Item = Self> {
        (self.get()..=u16::MAX).filter_map(Self::new)
    }
}

impl TryFrom<u16> for Port {
    type Error = DomainError;

    fn try_from(port: u16) -> Result<Self, Self::Error> {
        Self::new(port).ok_or(DomainError::InvalidPort(u32::from(port)))
    }
}

impl From<Port> for u16 {
    fn from(port: Port) -> Self {
        port.get()
    }
}

impl PartialEq<u16> for Port {
    fn eq(&self, other: &u16) -> bool {
        self.get() == *other
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Inclusive TCP port range for dedicated loopback listeners (worktree environments and
/// projects with a `LoopbackPort` address): unprivileged and not empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PortRangeRepr", into = "PortRangeRepr")]
pub struct PortRange {
    start: Port,
    end: Port,
}

impl PortRange {
    pub fn new(start: u16, end: u16) -> Result<Self, DomainError> {
        let invalid = || DomainError::InvalidSettings(format!("invalid port range {start}-{end}"));
        let (first, last) = (Port::new(start).ok_or_else(invalid)?, Port::new(end).ok_or_else(invalid)?);
        if !first.is_unprivileged() || last < first {
            return Err(invalid());
        }
        Ok(Self { start: first, end: last })
    }

    pub fn start(&self) -> Port {
        self.start
    }

    pub fn end(&self) -> Port {
        self.end
    }

    pub fn contains(&self, port: Port) -> bool {
        (self.start..=self.end).contains(&port)
    }

    /// Every port of the range, in ascending order.
    pub fn ports(&self) -> impl Iterator<Item = Port> {
        (self.start.get()..=self.end.get()).filter_map(Port::new)
    }
}

impl Default for PortRange {
    fn default() -> Self {
        Self { start: crate::address::LOOPBACK_PORT_RANGE_START, end: Port::of(8199) }
    }
}

impl fmt::Display for PortRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.start, self.end)
    }
}

/// Serialized form: `{ start = 8100, end = 8199 }`.
#[derive(Serialize, Deserialize)]
struct PortRangeRepr {
    start: u16,
    end: u16,
}

impl TryFrom<PortRangeRepr> for PortRange {
    type Error = DomainError;

    fn try_from(r: PortRangeRepr) -> Result<Self, Self::Error> {
        Self::new(r.start, r.end)
    }
}

impl From<PortRange> for PortRangeRepr {
    fn from(r: PortRange) -> Self {
        Self { start: r.start.get(), end: r.end.get() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_is_never_zero() {
        assert_eq!(Port::new(0), None);
        assert_eq!(Port::try_from(0), Err(DomainError::InvalidPort(0)));
        assert_eq!(Port::new(1).map(Port::get), Some(1));
        assert_eq!(Port::try_from(u16::MAX).map(u16::from), Ok(u16::MAX));
    }

    #[test]
    fn port_serializes_as_a_plain_integer() {
        assert_eq!(serde_json::to_string(&Port::of(8101)).unwrap(), "8101");
        assert_eq!(serde_json::from_str::<Port>("3306").unwrap(), Port::of(3306));
        assert!(serde_json::from_str::<Port>("0").is_err());
        assert!(serde_json::from_str::<Port>("65536").is_err());
        assert!(serde_json::from_str::<Port>("-1").is_err());
    }

    #[test]
    fn port_displays_and_compares_as_its_number() {
        assert_eq!(Port::of(8025).to_string(), "8025");
        assert_eq!(Port::of(80), 80);
        assert!(Port::of(1023) < Port::of(1024));
        assert!(!Port::of(1023).is_unprivileged());
        assert!(Port::of(FIRST_UNPRIVILEGED_PORT).is_unprivileged());
    }

    #[test]
    fn upward_starts_here_and_stops_at_the_last_port() {
        assert_eq!(Port::of(8025).upward().take(2).collect::<Vec<_>>(), [Port::of(8025), Port::of(8026)]);
        assert_eq!(Port::of(u16::MAX).upward().collect::<Vec<_>>(), [Port::of(u16::MAX)]);
    }

    #[test]
    fn range_validates_on_construction() {
        let r = PortRange::new(9000, 9099).unwrap();
        assert_eq!((r.start(), r.end()), (Port::of(9000), Port::of(9099)));
        assert!(PortRange::new(9000, 9000).is_ok());
        for (start, end) in [(9100, 9000), (1023, 2000), (0, 2000), (0, 0), (2000, 0)] {
            assert!(matches!(PortRange::new(start, end), Err(DomainError::InvalidSettings(_))), "{start}-{end}");
        }
    }

    #[test]
    fn range_contains_and_lists_its_ports() {
        let r = PortRange::new(3305, 3307).unwrap();
        assert!(r.contains(Port::of(3305)) && r.contains(Port::of(3307)));
        assert!(!r.contains(Port::of(3304)) && !r.contains(Port::of(3308)));
        assert_eq!(r.ports().map(Port::get).collect::<Vec<_>>(), [3305, 3306, 3307]);
        let top = PortRange::new(u16::MAX - 1, u16::MAX).unwrap();
        assert_eq!(top.ports().count(), 2);
        assert_eq!(r.to_string(), "3305-3307");
    }

    #[test]
    fn range_keeps_its_file_format_and_validates_it() {
        let r = PortRange::default();
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"start":8100,"end":8199}"#);
        assert_eq!(serde_json::from_str::<PortRange>(r#"{"start":8100,"end":8199}"#).unwrap(), r);
        assert!(serde_json::from_str::<PortRange>(r#"{"start":9100,"end":9000}"#).is_err());
        assert!(serde_json::from_str::<PortRange>(r#"{"start":80,"end":9000}"#).is_err());
    }
}
