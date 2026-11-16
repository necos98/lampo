//! Running the synchronous ports (the TOML store, generated files, the user PATH) from async
//! use cases without stalling the runtime's worker threads: a store write may retry for more
//! than a second while Windows holds the file, and a worker blocked meanwhile delays every other
//! task scheduled on it (GUI commands, control-pipe requests, service supervision).

use std::panic;

use tokio::runtime::{Handle, RuntimeFlavor};

use crate::error::{AppError, AppResult};

/// Runs `work` on the runtime's blocking thread pool (inline when no runtime is running). A
/// panic of `work` is resumed in the caller, as if it had run there.
pub(crate) async fn unblocked<T, F>(what: &str, work: F) -> AppResult<T>
where
    F: FnOnce() -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    if Handle::try_current().is_err() {
        return work();
    }
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(e) if e.is_panic() => panic::resume_unwind(e.into_panic()),
        Err(e) => Err(AppError::internal(format!("{what} was interrupted: {e}"))),
    }
}

/// Runs `work`, which borrows from the caller (so it cannot move to the blocking pool), telling a
/// multi-threaded runtime that this worker blocks: its other tasks move to another worker
/// meanwhile. On a current-thread runtime (the one-shot CLI) there is no other worker, and
/// `work` simply runs inline.
pub(crate) fn in_place<T>(work: impl FnOnce() -> T) -> T {
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => tokio::task::block_in_place(work),
        _ => work(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unblocked_returns_the_result_of_the_work() {
        assert_eq!(unblocked("reading", || Ok(7)).await.unwrap(), 7);
        let err = unblocked::<(), _>("reading", || Err(AppError::internal("locked"))).await.unwrap_err();
        assert_eq!(err.message, "locked");
    }

    #[tokio::test]
    #[should_panic(expected = "boom")]
    async fn unblocked_resumes_a_panic_in_the_caller() {
        let _ = unblocked::<(), _>("reading", || panic!("boom")).await;
    }

    #[test]
    fn unblocked_runs_inline_without_a_runtime() {
        let done = futures_util::FutureExt::now_or_never(unblocked("reading", || Ok(1)));
        assert_eq!(done.map(Result::unwrap), Some(1));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn in_place_runs_borrowing_work_on_a_multi_threaded_runtime() {
        let mut seen = Vec::new();
        in_place(|| seen.push(1));
        assert_eq!(seen, [1]);
    }

    #[tokio::test]
    async fn in_place_runs_inline_on_a_current_thread_runtime() {
        let mut seen = Vec::new();
        in_place(|| seen.push(1));
        assert_eq!(seen, [1]);
    }
}
