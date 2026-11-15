//! Web server configuration, php.ini and the hosts file. Implemented by `lampo-infra-web`.

use async_trait::async_trait;

use crate::error::PortResult;
use crate::hosts::HostsBlock;
use crate::web::{PhpIniSpec, WebServerConfig};

/// Renders and installs the Apache configuration (main config + one file per vhost).
#[async_trait]
pub trait WebServerConfigurator: Send + Sync {
    /// Renders `config` into a staging dir, validates it with `httpd -t`, then swaps it into
    /// `config.config_dir`. On validation failure nothing changes and
    /// `PortError::ConfigRejected { output }` is returned. Does not restart Apache.
    async fn apply(&self, config: &WebServerConfig) -> PortResult<()>;
}

/// Renders a project's php.ini.
pub trait PhpIniWriter: Send + Sync {
    /// Writes `<spec.output_dir>\php.ini` (atomically).
    fn write(&self, spec: &PhpIniSpec) -> PortResult<()>;
}

/// Reads the Lampo block of the hosts file (no elevation needed).
pub trait HostsFileReader: Send + Sync {
    /// `None` when the hosts file has no Lampo block.
    fn read_block(&self) -> PortResult<Option<HostsBlock>>;
}

/// Writes the Lampo block through the elevated helper (triggers a UAC prompt).
/// Callers must only invoke it when the block actually changed.
#[async_trait]
pub trait HostsFileWriter: Send + Sync {
    /// `PortError::ElevationDenied` if the user refuses the prompt.
    async fn write_block(&self, block: &HostsBlock) -> PortResult<()>;
}
