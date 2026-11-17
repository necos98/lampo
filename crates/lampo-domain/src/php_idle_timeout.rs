//! How long a site's idle `php-cgi.exe` stays alive before mod_fcgid closes it
//! (`FcgidIdleTimeout`). Short: sites nobody visits hold no RAM; long: fewer cold starts (about
//! a second) when switching between sites. Stored on disk as plain seconds
//! (`php_idle_timeout = 120`) and validated when read.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// Seconds a site's PHP process may stay idle, [`Self::MIN_SECS`]-[`Self::MAX_SECS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct PhpIdleTimeout(u32);

impl PhpIdleTimeout {
    /// Below this mod_fcgid would reap processes between the requests of one page load.
    pub const MIN_SECS: u32 = 30;
    /// One hour, the lifetime of a php-cgi.exe (`FcgidProcessLifeTime`): a longer idle time
    /// would never be reached.
    pub const MAX_SECS: u32 = 3600;
    /// Two minutes: low idle RAM while a site is being worked on.
    pub const DEFAULT: Self = Self(120);

    /// `InvalidSettings` outside [`Self::MIN_SECS`]-[`Self::MAX_SECS`].
    pub fn new(secs: u32) -> Result<Self, DomainError> {
        match (Self::MIN_SECS..=Self::MAX_SECS).contains(&secs) {
            true => Ok(Self(secs)),
            false => Err(DomainError::InvalidSettings(format!("the PHP idle timeout must be {}-{} seconds, got {secs}", Self::MIN_SECS, Self::MAX_SECS))),
        }
    }

    pub const fn secs(self) -> u32 {
        self.0
    }
}

impl Default for PhpIdleTimeout {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TryFrom<u32> for PhpIdleTimeout {
    type Error = DomainError;

    fn try_from(secs: u32) -> Result<Self, Self::Error> {
        Self::new(secs)
    }
}

impl From<PhpIdleTimeout> for u32 {
    fn from(timeout: PhpIdleTimeout) -> Self {
        timeout.0
    }
}

/// `120s`.
impl fmt::Display for PhpIdleTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}s", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_two_minutes() {
        assert_eq!(PhpIdleTimeout::default().secs(), 120);
    }

    #[test]
    fn accepts_the_bounds() {
        assert_eq!(PhpIdleTimeout::new(30).unwrap().secs(), 30);
        assert_eq!(PhpIdleTimeout::new(3600).unwrap().secs(), 3600);
    }

    #[test]
    fn rejects_values_out_of_range() {
        for secs in [0, 29, 3601, u32::MAX] {
            let err = PhpIdleTimeout::new(secs).unwrap_err();
            assert!(matches!(&err, DomainError::InvalidSettings(m) if m.contains("30-3600")), "{secs}: {err:?}");
        }
    }

    #[test]
    fn stored_as_plain_seconds_and_validated_on_read() {
        assert_eq!(serde_json::to_string(&PhpIdleTimeout::new(300).unwrap()).unwrap(), "300");
        assert_eq!(serde_json::from_str::<PhpIdleTimeout>("600").unwrap().secs(), 600);
        assert!(serde_json::from_str::<PhpIdleTimeout>("5").is_err());
    }

    #[test]
    fn displays_with_the_unit() {
        assert_eq!(PhpIdleTimeout::new(300).unwrap().to_string(), "300s");
    }
}
