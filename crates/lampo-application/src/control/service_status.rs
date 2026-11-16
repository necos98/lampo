//! Live service states read from the running app, for front-ends that do not own the service
//! processes (the CLI doctor).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::{ControlClient, ControlRequest};
use crate::context::CallContext;
use crate::dto::{ServiceStatusDto, StatusDto};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::use_cases::ServiceStatusSource;

/// A status read is quick: a hung app must not hang the caller.
pub const STATUS_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// The app's [`ControlRequest::Status`] services. `AppUnavailable` when the app is not running:
/// a read never starts it.
pub struct AppServiceStatus {
    client: Arc<ControlClient>,
}

impl AppServiceStatus {
    pub fn new(client: Arc<ControlClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ServiceStatusSource for AppServiceStatus {
    async fn statuses(&self) -> AppResult<Vec<ServiceStatusDto>> {
        if !self.client.is_app_running() {
            return Err(AppError::new(ErrorCode::AppUnavailable, "the Lampo app is not running"));
        }
        let status: StatusDto = self.client.call_within(ControlRequest::Status, CallContext::cli(false), STATUS_READ_TIMEOUT, &|_| {}).await?;
        Ok(status.services)
    }
}
