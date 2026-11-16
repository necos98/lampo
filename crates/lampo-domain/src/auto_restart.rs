//! Restarting a service that exited unexpectedly: how many attempts, how long to wait before
//! each one (exponential backoff), and when a service that kept running counts as stable again
//! (its attempts start over). Pure rules: the application's watchdog applies them.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The limits of the automatic restarts of one service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoRestartPolicy {
    /// Attempts in a row before giving up (the service then stays `Failed`).
    pub max_attempts: u32,
    /// Wait before the first attempt; doubled before each following one.
    pub first_delay: Duration,
    /// A service running this long after an automatic restart is stable: a later exit starts
    /// over from the first attempt.
    pub stable_after: Duration,
}

impl AutoRestartPolicy {
    /// 3 attempts after 2, 4 and 8 seconds; stable after a minute.
    pub const DEFAULT: Self = Self { max_attempts: 3, first_delay: Duration::from_secs(2), stable_after: Duration::from_secs(60) };

    /// The wait before `attempt` (1-based): `first_delay`, then doubled each time.
    pub fn delay(&self, attempt: u32) -> Duration {
        let doublings = attempt.saturating_sub(1).min(u32::BITS - 1);
        self.first_delay.saturating_mul(1u32 << doublings)
    }
}

impl Default for AutoRestartPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The attempts one service has used since it last ran stably.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RestartBudget {
    attempts: u32,
    /// When the last automatic restart got the service running (Unix ms).
    restarted_at_ms: Option<u64>,
}

impl RestartBudget {
    /// The number of the next attempt after an exit at `now_ms`, or `None` when the policy's
    /// attempts are used up. The attempts start over when the service ran for
    /// [`AutoRestartPolicy::stable_after`] since its last automatic restart.
    pub fn next_attempt(&mut self, policy: &AutoRestartPolicy, now_ms: u64) -> Option<u32> {
        let stable_ms = u64::try_from(policy.stable_after.as_millis()).unwrap_or(u64::MAX);
        if self.restarted_at_ms.is_some_and(|at| now_ms.saturating_sub(at) >= stable_ms) {
            *self = Self::default();
        }
        if self.attempts >= policy.max_attempts {
            return None;
        }
        self.attempts += 1;
        Some(self.attempts)
    }

    /// The attempt in progress got the service running at `now_ms`.
    pub fn restarted(&mut self, now_ms: u64) {
        self.restarted_at_ms = Some(now_ms);
    }

    /// Attempts used since the service last ran stably.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }
}

/// One step of the automatic restart of a service, published as
/// [`crate::events::DomainEvent::ServiceAutoRestart`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoRestartStep {
    /// The service exited unexpectedly: attempt `attempt` of `max_attempts` starts after `delay`.
    Scheduled { attempt: u32, max_attempts: u32, delay: Duration },
    /// Attempt `attempt` got the service running again.
    Restarted { attempt: u32 },
    /// Every attempt failed: the service stays `Failed`. `message` is the last failure.
    GaveUp { attempts: u32, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: AutoRestartPolicy = AutoRestartPolicy::DEFAULT;

    #[test]
    fn the_delay_doubles_from_two_seconds() {
        let delays: Vec<u64> = (1..=3).map(|a| POLICY.delay(a).as_secs()).collect();
        assert_eq!(delays, [2, 4, 8]);
        assert_eq!(POLICY.delay(0), POLICY.first_delay, "attempts are 1-based");
        assert!(POLICY.delay(u32::MAX) > POLICY.delay(3), "no overflow on absurd attempts");
    }

    #[test]
    fn three_attempts_then_none() {
        let mut budget = RestartBudget::default();
        let attempts: Vec<Option<u32>> = (0..4).map(|_| budget.next_attempt(&POLICY, 1_000)).collect();
        assert_eq!(attempts, [Some(1), Some(2), Some(3), None]);
        assert_eq!(budget.attempts(), 3);
    }

    #[test]
    fn a_service_that_ran_stably_starts_over() {
        let mut budget = RestartBudget::default();
        budget.next_attempt(&POLICY, 0);
        budget.restarted(2_000);
        assert_eq!(budget.next_attempt(&POLICY, 2_000 + 59_999), Some(2), "not stable yet");
        budget.restarted(70_000);
        assert_eq!(budget.next_attempt(&POLICY, 70_000 + 60_000), Some(1), "ran a full minute");
    }

    #[test]
    fn failed_attempts_after_a_stable_run_still_count() {
        let mut budget = RestartBudget::default();
        budget.next_attempt(&POLICY, 0);
        budget.restarted(0);
        // Stable: starts over once, then attempts that never get it running add up.
        let attempts: Vec<Option<u32>> = (0..4).map(|_| budget.next_attempt(&POLICY, 600_000)).collect();
        assert_eq!(attempts, [Some(1), Some(2), Some(3), None]);
    }

    #[test]
    fn a_clock_going_back_never_resets_the_attempts() {
        let mut budget = RestartBudget::default();
        budget.next_attempt(&POLICY, 100_000);
        budget.restarted(100_000);
        assert_eq!(budget.next_attempt(&POLICY, 1_000), Some(2));
    }
}
