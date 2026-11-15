//! What the last Apache apply of [`super::environment::ApplyEnvironment`] produced, and whether
//! the next sync must apply again.

use lampo_domain::PortError;
use lampo_domain::web::{PhpIniSpec, WebServerConfig};

use crate::error::AppError;

/// What the last Apache apply produced, so an unchanged configuration is not re-validated and a
/// still-broken site keeps being reported.
#[derive(Clone)]
pub(crate) struct WebState {
    pub desired: WebServerConfig,
    pub ini: Vec<PhpIniSpec>,
    pub outcome: WebOutcome,
}

impl WebState {
    /// `true` when applying `desired` with `ini` again would only repeat this outcome. A
    /// transient failure never counts: the next sync retries it.
    pub fn is_current(&self, desired: &WebServerConfig, ini: &[PhpIniSpec]) -> bool {
        !matches!(self.outcome, WebOutcome::Failed(_)) && self.desired == *desired && self.ini == ini
    }
}

/// How an Apache apply ended.
#[derive(Clone)]
pub(crate) enum WebOutcome {
    /// Applied; `exclusion` names the sites left out because they were rejected on their own.
    Applied { exclusion: Option<AppError> },
    /// Apache refused the configuration: kept, without re-validating, until it changes.
    Rejected(AppError),
    /// The apply did not run to the end (I/O, the validator not starting): retried next sync.
    Failed(AppError),
}

impl WebOutcome {
    pub fn from_port_error(error: PortError) -> Self {
        match error {
            PortError::ConfigRejected { .. } => Self::Rejected(error.into()),
            other => Self::Failed(other.into()),
        }
    }

    /// Apache must not be (re)started: the configuration on disk is not trustworthy.
    pub fn blocks_apache(&self) -> bool {
        !matches!(self, Self::Applied { .. })
    }

    pub fn error(&self) -> Option<&AppError> {
        match self {
            Self::Applied { exclusion } => exclusion.as_ref(),
            Self::Rejected(e) | Self::Failed(e) => Some(e),
        }
    }
}
