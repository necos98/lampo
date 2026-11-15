//! # lampo-infra-web
//!
//! The web tier. One reason to change: *how Apache, PHP and the hosts file are configured and
//! controlled on Windows*.
//!
//! | Adapter                  | Ports                                   |
//! |--------------------------|-----------------------------------------|
//! | [`ApacheConfigurator`]   | `WebServerConfigurator` (templates + `httpd -t` via `CommandRunner`) |
//! | [`ApacheRunner`]         | `ServiceRunner` for `ServiceKind::APACHE` (via `ProcessSupervisor`)  |
//! | [`PhpIniRenderer`]       | `PhpIniWriter`                          |
//! | [`ElevatedHostsFile`]    | `HostsFileReader`, `HostsFileWriter` (UAC via `lampo-hosts-helper.exe`) |
//!
//! Binary: `lampo-hosts-helper` — the only code that ever runs elevated. It splices the
//! requested `lampo_domain::hosts::HostsBlock` into the hosts file and nothing else (see
//! [`hosts::helper`] for its security rules).

pub mod apache;
pub mod hosts;
pub mod php_ini;

mod fs_util;
mod site_layout;
#[cfg(test)]
mod test_snapshot;

pub use apache::config::ApacheConfigurator;
pub use apache::render::{FcgidTuning, RenderOptions};
pub use apache::runner::ApacheRunner;
pub use hosts::ElevatedHostsFile;
pub use php_ini::PhpIniRenderer;
