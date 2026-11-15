//! Per-call context passed to mutating use cases by every front-end.

use serde::{Deserialize, Serialize};

/// Which front-end issued the call (logging, defaults).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    Gui,
    Cli,
}

/// Whether the call may trigger a UAC prompt (the elevated hosts-file helper).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ElevationPolicy {
    /// May show a UAC prompt (GUI actions, `lampo … --allow-elevation`).
    Allow,
    /// Never prompts. If the hosts block must change, everything else is still applied and the
    /// call fails with `ErrorCode::ElevationRequired` (CLI default).
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallContext {
    pub origin: Origin,
    pub elevation: ElevationPolicy,
}

impl CallContext {
    pub const GUI: CallContext = CallContext { origin: Origin::Gui, elevation: ElevationPolicy::Allow };

    pub fn cli(allow_elevation: bool) -> Self {
        Self { origin: Origin::Cli, elevation: if allow_elevation { ElevationPolicy::Allow } else { ElevationPolicy::Deny } }
    }
}
