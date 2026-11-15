//! Stopping the services when the app goes away by any path, not only the tray's "Esci" and an
//! update (which run `Shutdown` themselves): the end of the Windows session, an exit that
//! skipped them, a panic of the main thread. Without it the Job Object kills mysqld when the
//! process ends: InnoDB crash recovery at the next start, MyISAM `mysql.*` tables of 5.7 corrupted.
//!
//! Everything here blocks the calling thread (the main thread at `RunEvent::Exit`, the
//! session-end listener): never call it from an async task.

use std::sync::Once;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::composition;
use crate::state::AppState;

/// Graceful stop of every service at the end of the session or at exit. Short: Windows shows
/// the user that Lampo is blocking the session end meanwhile.
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(12);
/// How long a MySQL server asked to shut down by the last-resort stop may take to exit.
const MYSQL_LAST_RESORT_WAIT: Duration = Duration::from_secs(8);
/// Rust's name of the main thread, which runs the event loop.
const MAIN_THREAD: &str = "main";

static SERVICES_STOPPED: Once = Once::new();

/// Stops every service (bounded by [`GRACEFUL_STOP_TIMEOUT`]), then asks the MySQL servers
/// still running for a clean shutdown. Runs once per process: a concurrent caller (tao's
/// `RunEvent::Exit` and the session-end listener both react to `WM_ENDSESSION`) waits for the
/// first one, which keeps the process alive until the services are down. `cause` is logged.
pub fn stop_services(app: &AppHandle, cause: &str) {
    SERVICES_STOPPED.call_once(|| {
        stop_gracefully(app, cause);
        stop_mysql_last_resort(cause);
    });
}

fn stop_gracefully(app: &AppHandle, cause: &str) {
    let Some(state) = app.try_state::<AppState>() else { return };
    if state.shutdown.gate().is_closed() {
        log::info!("{cause}: the services were already stopped by a quit or an update");
        return;
    }
    log::info!("{cause}: stopping the services");
    let started = Instant::now();
    match tauri::async_runtime::block_on(tokio::time::timeout(GRACEFUL_STOP_TIMEOUT, state.shutdown.run())) {
        Ok(Ok(())) => log::info!("{cause}: services stopped in {:?}", started.elapsed()),
        Ok(Err(e)) => log::warn!("{cause}: stopping the services: {e}"),
        Err(_) => log::warn!("{cause}: services still stopping after {GRACEFUL_STOP_TIMEOUT:?}"),
    }
}

/// Synchronous and independent of the app state and the async runtime (safe in a panic hook).
fn stop_mysql_last_resort(cause: &str) {
    let stop = composition::stop_mysql_last_resort(MYSQL_LAST_RESORT_WAIT);
    if stop.found == 0 {
        return;
    }
    if stop.still_running == 0 {
        log::info!("{cause}: {} MySQL process(es) still running were shut down cleanly", stop.found);
    } else {
        log::warn!("{cause}: {} of {} MySQL process(es) still running after {MYSQL_LAST_RESORT_WAIT:?} ({} accepted the shutdown request)", stop.still_running, stop.found, stop.signalled);
    }
}

/// After the default report, a panic of the main thread (the event loop: the app is going
/// down) asks the MySQL servers for a clean shutdown before the Job Object kills them. Panics
/// of other threads leave the services alone: the async runtime catches those of its tasks and
/// the app keeps running.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        if std::thread::current().name() == Some(MAIN_THREAD) {
            log::error!("panic on the main thread: {info}");
            stop_mysql_last_resort("panic");
        }
    }));
}
