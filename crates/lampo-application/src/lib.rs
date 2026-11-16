//! # lampo-application
//!
//! Use cases and the DTOs front-ends exchange with them. Depends only on `lampo-domain`
//! (plus pure third-party crates). Every use case receives its ports as `Arc<dyn Port>`; the
//! composition roots (`apps/desktop/src-tauri`, `crates/lampo-cli`) are the only places that
//! name concrete adapters.
//!
//! * [`use_cases`] — one struct per capability;
//! * [`dto`] — wire types (GUI, `lampo --json`, control pipe);
//! * [`events`] — front-end event names and payloads;
//! * [`control`] — the CLI ↔ app protocol, dispatcher and typed client;
//! * [`tasks`] — background task ids, cancellation and progress events;
//! * [`context`] — per-call context (origin, elevation policy);
//! * [`error`] — [`AppError`] with stable [`ErrorCode`]s.

pub mod context;
pub mod control;
pub mod dto;
pub mod error;
pub mod events;
pub mod mapping;
pub mod tasks;
pub mod use_cases;

#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod ts_bindings;

pub use context::{CallContext, ElevationPolicy, Origin};
pub use error::{AppError, AppResult, ErrorCode};
