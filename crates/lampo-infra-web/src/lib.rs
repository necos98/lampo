//! # lampo-infra-web
//!
//! The web tier. One reason to change: *how Apache, PHP and the hosts file are configured and
//! controlled on Windows*, plus the other change that needs the same elevated helper: the
//! optional Defender exclusion of the MySQL data folder.
//!
//! | Adapter                  | Ports                                   |
//! |--------------------------|-----------------------------------------|
//! | [`ApacheConfigurator`]   | `WebServerConfigurator` (templates + `httpd -t` via `CommandRunner`) |
//! | [`ApacheRunner`]         | `ServiceRunner` for `ServiceKind::APACHE` (`lampo_infra_services::ServiceHost` with an httpd driver, via `ProcessSupervisor`) |
//! | [`PhpIniRenderer`]       | `PhpIniWriter`                          |
//! | [`ElevatedHostsFile`]    | `HostsFileReader`, `HostsFileWriter` (UAC via `lampo-hosts-helper.exe`) |
//! | [`DefenderScanExclusion`] | `ScanExclusionEditor` (UAC via `lampo-hosts-helper.exe`) |
//!
//! Binary: `lampo-hosts-helper` — the only code that ever runs elevated ([`elevation`]). It
//! splices the requested `lampo_domain::hosts::HostsBlock` into the hosts file (see
//! [`hosts::helper`]) or, in its Defender mode, adds or removes the exclusion of one fixed folder
//! (see [`defender::helper`]), and nothing else.

pub mod apache;
pub mod defender;
pub mod elevation;
pub mod hosts;
pub mod php_ini;

mod site_layout;
mod system_dir;
#[cfg(test)]
mod test_snapshot;

pub use apache::config::ApacheConfigurator;
pub use apache::render::{FcgidTuning, RenderOptions};
pub use apache::runner::ApacheRunner;
pub use defender::DefenderScanExclusion;
pub use hosts::ElevatedHostsFile;
pub use php_ini::PhpIniRenderer;
