//! # lampo-domain
//!
//! The innermost layer of Lampo. It contains:
//!
//! * entities and value objects ([`project::Project`], [`php::PhpMinor`], [`package::CatalogEntry`], …),
//!   each validating its own invariants on construction;
//! * pure domain rules (which services must run, how the hosts-file block is rewritten, how the
//!   CLI shim resolves a project, which Visual C++ runtime a PHP build needs, …);
//! * the error types shared by every layer ([`error::DomainError`], [`error::PortError`]);
//! * every **port trait** ([`ports`]) that the application layer depends on and the infra crates implement.
//!
//! Rules for this crate (enforced by review, keep them):
//! * no IO of any kind (no filesystem, network, processes, clock, environment variables);
//! * no dependency on any other workspace crate;
//! * paths are only *joined and compared*, never touched.

pub mod address;
pub mod apache_directives;
pub mod auto_restart;
pub mod claude_code;
pub mod detection;
pub mod env_var;
pub mod error;
pub mod events;
pub mod hostname;
pub mod hosts;
pub mod logs;
pub mod mysql;
pub mod package;
pub mod paths;
pub mod php;
pub mod ports;
pub mod preset;
pub mod progress;
pub mod project;
pub mod project_file;
pub mod project_import;
pub mod resolution;
pub mod scaffold;
pub mod service;
pub mod settings;
pub mod shim;
pub mod site;
pub mod slug;
pub mod sql_dump;
pub mod tcp_port;
pub mod vc_runtime;
pub mod version;
pub mod web;
pub mod worktree;

pub use error::{DomainError, PortError, PortResult};
