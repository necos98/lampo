//! Stale-while-revalidate cache of one live catalog source.
//!
//! A listing never waits for the network when the source has something to answer with: the
//! entries of the last refresh, or failing that what the source keeps locally (disk cache,
//! snapshot embedded at build time). When those are older than the TTL, one refresh runs in a
//! background task while callers keep getting the stale entries. Only a source that has nothing
//! at all (first run, no disk cache) makes its first callers wait for that refresh.
//!
//! The state lock is held only to read or swap the state, never across network or disk I/O. A
//! failed refresh keeps the previous entries and is not retried for [`FAILURE_RETRY`]. Dropping
//! the cache cancels a refresh still pausing between attempts.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use lampo_domain::package::CatalogEntry;
use lampo_domain::progress::CancelFlag;
use lampo_domain::{PortError, PortResult};
use tokio::sync::watch;

/// After a failed refresh, the network is tried again this soon (stale entries are served meanwhile).
pub(crate) const FAILURE_RETRY: Duration = Duration::from_secs(60);

/// What the cache needs from a live source.
#[async_trait]
pub(crate) trait LiveCatalog: Send + Sync + 'static {
    /// Entries the source keeps locally (disk cache, embedded snapshot), read without the
    /// network; `None` when it keeps nothing usable.
    async fn stored(&self) -> Option<Vec<CatalogEntry>>;
    /// Entries read from the network, persisted for [`LiveCatalog::stored`] where the source
    /// keeps a disk cache. Stops between attempts once `cancel` is set.
    async fn fetch_live(&self, cancel: &CancelFlag) -> PortResult<Vec<CatalogEntry>>;
}

pub(crate) struct RevalidatingCache {
    name: &'static str,
    ttl: Duration,
    shared: Arc<Shared>,
    cancel: CancelFlag,
}

struct Shared {
    state: Mutex<State>,
    /// Bumped after every refresh, so callers with nothing to answer can wait for one.
    refreshes: watch::Sender<u64>,
}

#[derive(Default)]
struct State {
    /// The last refreshed entries, or the stored ones until the first refresh succeeds.
    entries: Option<Vec<CatalogEntry>>,
    /// [`LiveCatalog::stored`] was read (once per cache).
    seeded: bool,
    /// Until when `entries` need no refresh; `None` while they are not from a refresh.
    fresh_until: Option<Instant>,
    /// No refresh starts before this (set by a failure).
    retry_after: Option<Instant>,
    refreshing: bool,
    last_error: Option<PortError>,
}

impl RevalidatingCache {
    /// `name` labels the logs; refreshed entries are fresh for `ttl`.
    pub(crate) fn new(name: &'static str, ttl: Duration) -> Self {
        let shared = Shared { state: Mutex::new(State::default()), refreshes: watch::Sender::new(0) };
        Self { name, ttl, shared: Arc::new(shared), cancel: CancelFlag::new() }
    }

    /// The entries to answer with now, starting a background refresh when they are stale.
    /// Waits for the network only while neither a refresh nor the source's local copy has
    /// produced anything yet; then fails with the refresh's error.
    pub(crate) async fn get<S: LiveCatalog>(&self, source: &Arc<S>) -> PortResult<Vec<CatalogEntry>> {
        self.seed(source.as_ref()).await;
        // Subscribed before reading the state, so a refresh ending in between is not missed.
        let mut refreshes = self.shared.refreshes.subscribe();
        {
            let mut state = self.shared.lock();
            let now = Instant::now();
            if let (Some(entries), Some(fresh_until)) = (&state.entries, state.fresh_until) {
                if now < fresh_until {
                    return Ok(entries.clone());
                }
            }
            if !state.refreshing && state.retry_after.is_none_or(|at| now >= at) {
                state.refreshing = true;
                self.spawn_refresh(source.clone());
            }
            if let Some(entries) = &state.entries {
                return Ok(entries.clone());
            }
            if !state.refreshing {
                return Err(state.unavailable(self.name));
            }
        }
        // Nothing to answer with yet: wait for the refresh under way. The sender lives in
        // `self.shared`, so `changed` can only end by a refresh finishing.
        let _ = refreshes.changed().await;
        let state = self.shared.lock();
        state.entries.clone().ok_or_else(|| state.unavailable(self.name))
    }

    /// Loads the source's local copy the first time the cache is used (no lock held while reading).
    async fn seed<S: LiveCatalog>(&self, source: &S) {
        if self.shared.lock().seeded {
            return;
        }
        let stored = source.stored().await;
        let mut state = self.shared.lock();
        if !state.seeded {
            state.seeded = true;
            if state.entries.is_none() {
                state.entries = stored;
            }
        }
    }

    fn spawn_refresh<S: LiveCatalog>(&self, source: Arc<S>) {
        let in_flight = InFlight { name: self.name, ttl: self.ttl, shared: self.shared.clone(), outcome: None };
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            let outcome = source.fetch_live(&cancel).await;
            in_flight.complete(outcome);
        });
    }

    /// Waits until no refresh is under way (tests).
    #[cfg(test)]
    pub(crate) async fn settled(&self) {
        loop {
            let mut refreshes = self.shared.refreshes.subscribe();
            if !self.shared.lock().refreshing {
                return;
            }
            let _ = refreshes.changed().await;
        }
    }
}

impl Drop for RevalidatingCache {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // The state is replaced field by field under the lock: a panic elsewhere cannot leave it torn.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl State {
    fn unavailable(&self, name: &str) -> PortError {
        self.last_error.clone().unwrap_or_else(|| PortError::Network(format!("{name}: catalog not loaded yet")))
    }
}

/// The refresh task's handle on the state: records the outcome when dropped, so the state never
/// stays "refreshing" even if the task panics or the runtime shuts down mid-refresh.
struct InFlight {
    name: &'static str,
    ttl: Duration,
    shared: Arc<Shared>,
    outcome: Option<PortResult<Vec<CatalogEntry>>>,
}

impl InFlight {
    /// Records the refresh's outcome (applied by `drop`).
    fn complete(mut self, outcome: PortResult<Vec<CatalogEntry>>) {
        self.outcome = Some(outcome);
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        let outcome = self.outcome.take().unwrap_or(Err(PortError::Cancelled));
        let now = Instant::now();
        {
            let mut state = self.shared.lock();
            match outcome {
                Ok(entries) => {
                    state.entries = Some(entries);
                    state.fresh_until = Some(now + self.ttl);
                    state.retry_after = None;
                    state.last_error = None;
                }
                Err(e) => {
                    tracing::warn!(source = self.name, error = %e, "cannot refresh the live catalog; keeping the entries already known");
                    state.retry_after = Some(now + FAILURE_RETRY);
                    state.last_error = Some(e);
                }
            }
            state.refreshing = false;
        }
        self.shared.refreshes.send_modify(|n| *n = n.wrapping_add(1));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use lampo_domain::package::{CatalogChannel, DownloadUrl, PackageId, PackageKind, PackagePayload};
    use lampo_domain::version::PackageVersion;
    use tokio::sync::Semaphore;

    use super::*;

    const TTL: Duration = Duration::from_secs(3600);
    /// Upper bound for anything that must not wait on the gated network.
    const PROMPT: Duration = Duration::from_millis(500);

    fn entry(version: &str) -> CatalogEntry {
        CatalogEntry {
            id: PackageId::new(PackageKind::COMPOSER, PackageVersion::parse(version).unwrap()),
            url: DownloadUrl::new(format!("https://getcomposer.org/download/{version}/composer.phar")).unwrap(),
            sha256: None,
            size_bytes: None,
            vc_runtime: None,
            channel: CatalogChannel::Live,
            end_of_life: false,
            payload: PackagePayload::SingleFile { file_name: "composer.phar".into() },
        }
    }

    /// A source whose network answers only when the test releases the gate.
    struct Gated {
        stored: Option<Vec<CatalogEntry>>,
        live: PortResult<Vec<CatalogEntry>>,
        gate: Semaphore,
        calls: AtomicUsize,
    }

    impl Gated {
        fn new(stored: Option<Vec<CatalogEntry>>, live: PortResult<Vec<CatalogEntry>>) -> Arc<Self> {
            Arc::new(Self { stored, live, gate: Semaphore::new(0), calls: AtomicUsize::new(0) })
        }

        fn open(&self) {
            self.gate.add_permits(Semaphore::MAX_PERMITS);
        }
    }

    #[async_trait]
    impl LiveCatalog for Gated {
        async fn stored(&self) -> Option<Vec<CatalogEntry>> {
            self.stored.clone()
        }

        async fn fetch_live(&self, _cancel: &CancelFlag) -> PortResult<Vec<CatalogEntry>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let _permit = self.gate.acquire().await.unwrap();
            self.live.clone()
        }
    }

    async fn get_promptly(cache: &RevalidatingCache, source: &Arc<Gated>) -> PortResult<Vec<CatalogEntry>> {
        tokio::time::timeout(PROMPT, cache.get(source)).await.expect("answered without waiting for the network")
    }

    #[tokio::test]
    async fn answers_with_the_stored_entries_while_refreshing_in_background() {
        let source = Gated::new(Some(vec![entry("2.0.0")]), Ok(vec![entry("2.1.0")]));
        let cache = RevalidatingCache::new("test", TTL);
        assert_eq!(get_promptly(&cache, &source).await.unwrap(), [entry("2.0.0")]);
        assert_eq!(get_promptly(&cache, &source).await.unwrap(), [entry("2.0.0")], "still stale, not blocked");

        source.open();
        cache.settled().await;
        assert_eq!(source.calls.load(Ordering::SeqCst), 1, "one refresh for both callers");
        assert_eq!(get_promptly(&cache, &source).await.unwrap(), [entry("2.1.0")]);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1, "fresh entries need no new refresh");
    }

    #[tokio::test]
    async fn a_failed_refresh_keeps_the_stale_entries_and_backs_off() {
        let source = Gated::new(Some(vec![entry("2.0.0")]), Err(PortError::Network("offline".into())));
        source.open();
        let cache = RevalidatingCache::new("test", TTL);
        assert_eq!(get_promptly(&cache, &source).await.unwrap(), [entry("2.0.0")]);
        cache.settled().await;
        assert_eq!(get_promptly(&cache, &source).await.unwrap(), [entry("2.0.0")]);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1, "no new attempt before FAILURE_RETRY");
    }

    #[tokio::test]
    async fn without_stored_entries_the_first_caller_waits_for_the_refresh() {
        let source = Gated::new(None, Ok(vec![entry("2.1.0")]));
        source.open();
        let cache = RevalidatingCache::new("test", TTL);
        assert_eq!(cache.get(&source).await.unwrap(), [entry("2.1.0")]);
    }

    #[tokio::test]
    async fn without_stored_entries_a_failure_is_reported_then_remembered() {
        let source = Gated::new(None, Err(PortError::Network("offline".into())));
        source.open();
        let cache = RevalidatingCache::new("test", TTL);
        assert_eq!(cache.get(&source).await.unwrap_err(), PortError::Network("offline".into()));
        assert_eq!(get_promptly(&cache, &source).await.unwrap_err(), PortError::Network("offline".into()));
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dropping_the_cache_cancels_the_refresh() {
        struct Cancellable;
        #[async_trait]
        impl LiveCatalog for Cancellable {
            async fn stored(&self) -> Option<Vec<CatalogEntry>> {
                Some(Vec::new())
            }
            async fn fetch_live(&self, cancel: &CancelFlag) -> PortResult<Vec<CatalogEntry>> {
                crate::http::sleep_cancellable(Duration::from_secs(60), cancel).await?;
                Ok(Vec::new())
            }
        }
        let cache = RevalidatingCache::new("test", TTL);
        let shared = cache.shared.clone();
        let mut refreshes = shared.refreshes.subscribe();
        cache.get(&Arc::new(Cancellable)).await.unwrap();
        drop(cache);
        tokio::time::timeout(PROMPT, refreshes.changed()).await.expect("the refresh ended").unwrap();
        assert_eq!(shared.lock().last_error, Some(PortError::Cancelled));
    }
}
