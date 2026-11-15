//! System facts the domain cannot compute itself.

use std::time::Duration;

use async_trait::async_trait;

/// Wall-clock time. Implemented by `lampo-infra-store` (`SystemClock`); tests use a fixed clock.
pub trait Clock: Send + Sync {
    fn now_unix_ms(&self) -> u64;
}

/// Waiting without blocking the runtime (polling loops). Implemented by `lampo-infra-services`
/// (`TokioTimer`); tests advance a fake [`Clock`] instead of waiting.
#[async_trait]
pub trait Timer: Send + Sync {
    async fn sleep(&self, duration: Duration);
}
