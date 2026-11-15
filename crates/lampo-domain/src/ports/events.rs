//! Outbound notifications: domain events and task progress.

use crate::events::DomainEvent;
use crate::progress::Progress;

/// Publishes domain events to whoever hosts the core (the GUI, a log…).
///
/// Must be cheap and non-blocking: implementations enqueue/emit and return immediately.
/// Publishing never fails from the caller's point of view (a closed GUI is not an error).
pub trait EventPublisher: Send + Sync {
    fn publish(&self, event: DomainEvent);
}

/// Receives progress of one long operation. Same contract as [`EventPublisher`]: cheap,
/// non-blocking, infallible. Adapters should throttle (e.g. at most ~10 reports per second).
pub trait ProgressSink: Send + Sync {
    fn report(&self, progress: Progress);
}

/// Discards events (CLI, tests).
#[derive(Debug, Default, Clone, Copy)]
pub struct NullEventPublisher;

impl EventPublisher for NullEventPublisher {
    fn publish(&self, _event: DomainEvent) {}
}

/// Discards progress (CLI, tests).
#[derive(Debug, Default, Clone, Copy)]
pub struct NullProgressSink;

impl ProgressSink for NullProgressSink {
    fn report(&self, _progress: Progress) {}
}
