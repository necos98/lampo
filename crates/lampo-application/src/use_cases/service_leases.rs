//! Leases that keep a service as it is while a long operation (a database import, a worktree
//! database clone) uses it: the environment sync postpones stopping or restarting a leased
//! service, and applies the postponed change once the last lease is released.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use lampo_domain::service::ServiceId;
use tokio::sync::Notify;

/// How many leases a service has, and whether a sync postponed a change for it.
#[derive(Default)]
struct Held {
    count: usize,
    deferred: bool,
}

/// The leases of every service, shared by the environment sync and the leases it hands out.
#[derive(Default)]
pub(crate) struct ServiceLeases {
    held: Mutex<HashMap<ServiceId, Held>>,
    /// Signalled when the last lease of a service with a postponed change is released.
    deferred_released: Notify,
}

impl ServiceLeases {
    fn held(&self) -> MutexGuard<'_, HashMap<ServiceId, Held>> {
        self.held.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// One more lease of `service`, released when the returned guard is dropped.
    pub fn acquire(self: &Arc<Self>, service: ServiceId) -> ServiceLease {
        self.held().entry(service.clone()).or_default().count += 1;
        ServiceLease { leases: Arc::clone(self), service }
    }

    /// `true` when `service` is leased.
    pub fn is_leased(&self, service: &ServiceId) -> bool {
        self.held().contains_key(service)
    }

    /// `true` when `service` is leased: the caller must leave it as it is, and the postponed
    /// change is announced by [`Self::deferred_released`] once the last lease is gone.
    pub fn defer(&self, service: &ServiceId) -> bool {
        match self.held().get_mut(service) {
            Some(held) => {
                held.deferred = true;
                true
            }
            None => false,
        }
    }

    /// Completes after the last lease of a service with a postponed change was released (at
    /// once if that happened since the previous call).
    pub async fn deferred_released(&self) {
        self.deferred_released.notified().await;
    }

    fn release(&self, service: &ServiceId) {
        let mut held = self.held();
        let Some(entry) = held.get_mut(service) else { return };
        entry.count -= 1;
        if entry.count > 0 {
            return;
        }
        let deferred = entry.deferred;
        held.remove(service);
        drop(held);
        if deferred {
            self.deferred_released.notify_one();
        }
    }
}

/// Keeps a service running while it is held: the environment sync neither stops nor restarts
/// it, and manual stops/restarts are refused. Dropping it releases the lease.
#[must_use = "the service is only protected while the lease is held"]
pub struct ServiceLease {
    leases: Arc<ServiceLeases>,
    service: ServiceId,
}

impl Drop for ServiceLease {
    fn drop(&mut self) {
        self.leases.release(&self.service);
    }
}

impl std::fmt::Debug for ServiceLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceLease").field("service", &self.service).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::mysql_id;

    #[test]
    fn a_service_is_leased_until_its_last_lease_is_dropped() {
        let leases = Arc::new(ServiceLeases::default());
        let id = mysql_id("8.4");
        let first = leases.acquire(id.clone());
        let second = leases.acquire(id.clone());
        drop(first);
        assert!(leases.is_leased(&id));
        drop(second);
        assert!(!leases.is_leased(&id));
        assert!(!leases.defer(&id), "nothing to postpone once released");
    }

    #[tokio::test]
    async fn releasing_a_deferred_service_signals_once() {
        let leases = Arc::new(ServiceLeases::default());
        let id = mysql_id("8.4");
        let lease = leases.acquire(id.clone());
        assert!(leases.defer(&id));
        drop(lease);
        // The permit was stored: this completes without waiting.
        leases.deferred_released().await;
        assert!(!leases.is_leased(&id));
    }
}
