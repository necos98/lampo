//! [`ServiceLifecycle`] for [`ApplyEnvironment`]: one service outside a full sync, under the
//! same lock, except for giving up a start that is still waiting for readiness.

use async_trait::async_trait;
use lampo_domain::ports::ServiceRunner;
use lampo_domain::service::{ServiceId, ServiceState};

use super::{ApplyEnvironment, ServiceLifecycle, SyncState, is_up};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::use_cases::service_leases::ServiceLease;

impl ApplyEnvironment {
    /// [`ServiceLifecycle::ensure`], with the lock held.
    pub(super) async fn ensure_locked(&self, state: &mut SyncState, id: &ServiceId) -> AppResult<bool> {
        let runner = self.runner(id)?;
        if is_up(runner.status(id).state) {
            return Ok(false);
        }
        let launch = self.current_launch(id)?;
        self.start_with(runner.as_ref(), &launch, &launch.ports()).await?;
        state.started(id.clone(), launch);
        self.provision_started_mysql(state, id).await;
        Ok(true)
    }

    /// `Conflict` while `id` is leased.
    fn refuse_if_leased(&self, id: &ServiceId, action: &str) -> AppResult<()> {
        if !self.leases.is_leased(id) {
            return Ok(());
        }
        Err(AppError::new(ErrorCode::Conflict, format!("{id} is in use by a running database operation (import, export, clone): {action} it when the operation finishes")))
    }

    /// A start still waiting for readiness (a MySQL recovery can take up to an hour) holds the
    /// lock: the runner is told to give it up before the lock is taken, or the stop would wait
    /// for it. `true` when `id` was starting.
    async fn give_up_start(runner: &dyn ServiceRunner, id: &ServiceId) -> AppResult<bool> {
        if runner.status(id).state != ServiceState::Starting {
            return Ok(false);
        }
        runner.stop(id).await?;
        Ok(true)
    }
}

#[async_trait]
impl ServiceLifecycle for ApplyEnvironment {
    async fn ensure(&self, service: &ServiceId) -> AppResult<bool> {
        let mut state = self.lock.lock().await;
        self.ensure_locked(&mut state, service).await
    }

    async fn lease(&self, service: &ServiceId) -> AppResult<ServiceLease> {
        let mut state = self.lock.lock().await;
        self.ensure_locked(&mut state, service).await?;
        // The port of the running instance: a sync may have postponed its restart onto a new one
        // while another lease holds it.
        let port = match state.launches.get(service) {
            Some(launch) => launch.port,
            None => self.current_launch(service)?.port,
        };
        // Taken before the lock is released: no sync can slip in between.
        Ok(self.leases.acquire(service.clone(), port))
    }

    async fn restart(&self, service: &ServiceId) -> AppResult<()> {
        let mut state = self.lock.lock().await;
        self.refuse_if_leased(service, "restart")?;
        let runner = self.runner(service)?;
        let launch = self.current_launch(service)?;
        self.restart_with(runner.as_ref(), &launch).await?;
        state.launches.insert(service.clone(), launch);
        Ok(())
    }

    async fn stop(&self, service: &ServiceId) -> AppResult<bool> {
        let runner = self.runner(service)?;
        self.refuse_if_leased(service, "stop")?;
        let gave_up = Self::give_up_start(runner.as_ref(), service).await?;
        let mut state = self.lock.lock().await;
        // Checked again: a lease is only taken under the lock.
        self.refuse_if_leased(service, "stop")?;
        let running = runner.status(service).state != ServiceState::Stopped;
        if running {
            runner.stop(service).await?;
        }
        state.forget(service);
        Ok(gave_up || running)
    }
}
