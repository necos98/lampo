//! `Timer` on the tokio runtime.

use std::time::Duration;

use async_trait::async_trait;
use lampo_domain::ports::Timer;

#[derive(Debug, Default, Clone, Copy)]
pub struct TokioTimer;

#[async_trait]
impl Timer for TokioTimer {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}
