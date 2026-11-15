//! When a service start gives up. A fixed timeout kills a server that is merely slow (mysqld
//! during InnoDB crash recovery or a data dictionary upgrade), and every new attempt starts the
//! work over: so while the start keeps making visible progress the deadline moves forward.

use std::time::Duration;

use tokio::time::Instant;

/// `timeout` after the start, or after the last change of the progress marker when there is
/// one; never later than `limit` after the start.
#[derive(Debug, Clone)]
pub(crate) struct StartupDeadline {
    started: Instant,
    timeout: Duration,
    limit: Duration,
    progress: Option<u64>,
    deadline: Instant,
}

impl StartupDeadline {
    pub(crate) fn new(now: Instant, timeout: Duration, limit: Duration, progress: Option<u64>) -> Self {
        Self { started: now, timeout, limit, progress, deadline: now + timeout }
    }

    /// Records the current progress marker: a change pushes the deadline to `timeout` from now.
    pub(crate) fn observe(&mut self, now: Instant, progress: Option<u64>) {
        if progress.is_none() || progress == self.progress {
            return;
        }
        self.progress = progress;
        self.deadline = (now + self.timeout).min(self.started + self.limit);
    }

    pub(crate) fn expired(&self, now: Instant) -> bool {
        now >= self.deadline
    }

    /// The start has run past its first deadline and goes on only because it makes progress.
    pub(crate) fn is_extended(&self, now: Instant) -> bool {
        now >= self.started + self.timeout
    }

    pub(crate) fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(120);
    const LIMIT: Duration = Duration::from_secs(3600);

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn without_a_marker_the_deadline_is_fixed() {
        let t0 = Instant::now();
        let mut d = StartupDeadline::new(t0, TIMEOUT, LIMIT, None);
        d.observe(t0 + secs(100), None);
        assert!(!d.expired(t0 + secs(119)));
        assert!(d.expired(t0 + secs(120)));
    }

    #[test]
    fn progress_pushes_the_deadline_from_the_last_change() {
        let t0 = Instant::now();
        let mut d = StartupDeadline::new(t0, TIMEOUT, LIMIT, Some(10));
        d.observe(t0 + secs(100), Some(10));
        assert!(d.expired(t0 + secs(120)), "an unchanged marker is no progress");

        let mut d = StartupDeadline::new(t0, TIMEOUT, LIMIT, Some(10));
        d.observe(t0 + secs(100), Some(20));
        assert!(!d.expired(t0 + secs(219)));
        assert!(d.is_extended(t0 + secs(130)));
        assert!(!d.is_extended(t0 + secs(119)));
        assert!(d.expired(t0 + secs(220)));
        assert_eq!(d.elapsed(t0 + secs(220)), secs(220));
    }

    #[test]
    fn progress_never_extends_past_the_limit() {
        let t0 = Instant::now();
        let mut d = StartupDeadline::new(t0, TIMEOUT, LIMIT, Some(0));
        d.observe(t0 + secs(3590), Some(1));
        assert!(d.expired(t0 + LIMIT));
    }
}
