//! [`CrashRecovery`] for [`ApplyEnvironment`]: a crashed service starts again under the
//! lifecycle lock, unless a stop (the user's or a sync's) removed its launch since it started.

use async_trait::async_trait;
use lampo_domain::service::ServiceId;

use super::ApplyEnvironment;
use crate::error::AppResult;
use crate::use_cases::auto_restart::{CrashRecovery, Revival};

#[async_trait]
impl CrashRecovery for ApplyEnvironment {
    async fn revive(&self, service: &ServiceId) -> AppResult<Revival> {
        let mut state = self.lock.lock().await;
        // Every start records its launch and every stop removes it.
        if !state.launches.contains_key(service) {
            return Ok(Revival::NoLongerWanted);
        }
        Ok(match self.ensure_locked(&mut state, service).await? {
            true => Revival::Restarted,
            false => Revival::AlreadyUp,
        })
    }
}
