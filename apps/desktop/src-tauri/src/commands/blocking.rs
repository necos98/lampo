//! Keeps blocking work off the main thread. Tauri v2 runs non-`async` commands on the main
//! thread (the event loop): disk or process I/O there freezes the window ("Non risponde"). Every
//! command that touches the disk, the registry or other processes is `async` and hands its
//! synchronous use case to [`run_blocking`].

use std::any::Any;
use std::panic::{self, AssertUnwindSafe};

use lampo_application::{AppError, AppResult};

/// Runs `work` on the blocking thread pool of the async runtime. A panic of `work` becomes
/// `AppError::internal` instead of an unanswered `invoke`.
pub async fn run_blocking<T, F>(work: F) -> AppResult<T>
where
    F: FnOnce() -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    let joined = tauri::async_runtime::spawn_blocking(move || panic::catch_unwind(AssertUnwindSafe(work))).await;
    match joined {
        Ok(Ok(result)) => result,
        Ok(Err(payload)) => Err(panic_error(payload.as_ref())),
        Err(e) => Err(AppError::internal(format!("operazione interrotta: {e}"))),
    }
}

/// The error reported for a panic, with its message when the payload carries one.
pub fn panic_error(payload: &(dyn Any + Send)) -> AppError {
    let detail = payload.downcast_ref::<&str>().copied().or_else(|| payload.downcast_ref::<String>().map(String::as_str));
    match detail {
        Some(message) => AppError::internal(format!("errore interno imprevisto: {message}")),
        None => AppError::internal("errore interno imprevisto"),
    }
}

#[cfg(test)]
mod tests {
    use lampo_application::ErrorCode;

    use super::*;

    #[test]
    fn returns_the_result_of_the_work() {
        let result = tauri::async_runtime::block_on(run_blocking(|| Ok(42)));
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn propagates_the_error_of_the_work() {
        let result: AppResult<()> = tauri::async_runtime::block_on(run_blocking(|| Err(AppError::internal("disco pieno"))));
        assert_eq!(result.unwrap_err().message, "disco pieno");
    }

    #[test]
    fn turns_a_panic_into_an_internal_error() {
        let result: AppResult<()> = tauri::async_runtime::block_on(run_blocking(|| panic!("indice fuori range")));
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::Internal);
        assert!(error.message.contains("indice fuori range"), "{}", error.message);
    }

    #[test]
    fn panic_error_reads_string_payloads_and_tolerates_others() {
        assert!(panic_error(&String::from("formattato 7")).message.contains("formattato 7"));
        assert_eq!(panic_error(&7_u8).message, "errore interno imprevisto");
    }
}
