//! End of the Windows session (logoff, shutdown, restart) while Lampo runs.
//!
//! Windows sends `WM_QUERYENDSESSION` / `WM_ENDSESSION` to top-level windows only (message-only
//! windows never get them), so a hidden top-level window on a thread of its own listens:
//! * `WM_QUERYENDSESSION`: registers a shutdown block reason (Windows then waits for the app and
//!   tells the user why) and lets the session end;
//! * `WM_ENDSESSION` (ending): runs the stop callback, then releases the block reason; once it
//!   returns, Windows terminates the process (and the Job Object kills whatever is left);
//! * `WM_ENDSESSION` (cancelled): releases the block reason.
//!
//! tao's own window also reacts to `WM_ENDSESSION`, emitting `RunEvent::Exit` on the main
//! thread: the callback must be safe to run concurrently with that (see [`crate::orderly_exit`]).

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, TRUE, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DispatchMessageW, ENDSESSION_LOGOFF, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, MSG, PostQuitMessage, RegisterClassExW,
    SetWindowLongPtrW, TranslateMessage, WM_DESTROY, WM_ENDSESSION, WM_NCCREATE, WM_NCDESTROY, WM_QUERYENDSESSION, WNDCLASSEXW, WS_OVERLAPPED,
};

const WINDOW_CLASS: &str = "LampoSessionEnd";
const THREAD_NAME: &str = "lampo-session-end";

/// What the window does at the end of the session; owned by the window (`GWLP_USERDATA`).
struct Handler {
    /// NUL-terminated UTF-16 block reason.
    reason: Vec<u16>,
    on_session_end: Box<dyn Fn() + Send>,
}

impl Handler {
    fn block(&self, hwnd: HWND, flags: LPARAM) {
        let kind = if flags as u32 & ENDSESSION_LOGOFF != 0 { "logoff" } else { "shutdown or restart" };
        log::info!("session end ({kind}) requested: services will be stopped before Windows ends the session");
        // SAFETY: `hwnd` is this thread's window, `reason` is NUL-terminated.
        if unsafe { ShutdownBlockReasonCreate(hwnd, self.reason.as_ptr()) } == 0 {
            log::warn!("session end: cannot register the shutdown block reason: {}", io::Error::last_os_error());
        }
    }

    fn end(&self, hwnd: HWND, ending: bool) {
        if ending {
            // A panic must not cross the window procedure (that aborts before anything stops).
            if catch_unwind(AssertUnwindSafe(|| (self.on_session_end)())).is_err() {
                log::error!("session end: stopping the services panicked");
            }
        } else {
            log::info!("session end cancelled: Lampo keeps running");
        }
        // SAFETY: `hwnd` is this thread's window; failure only means no reason was registered.
        unsafe { ShutdownBlockReasonDestroy(hwnd) };
    }
}

/// Starts listening for the end of the session: `on_session_end` runs on the listener's thread
/// when the session actually ends, with `reason` shown by Windows while it runs.
pub fn watch(reason: &str, on_session_end: impl Fn() + Send + 'static) -> io::Result<()> {
    spawn_window(reason, Box::new(on_session_end)).map(|_| ())
}

/// The listener thread and its window (raw `HWND` as `isize`: `HWND` is not `Send`).
fn spawn_window(reason: &str, on_session_end: Box<dyn Fn() + Send>) -> io::Result<(isize, JoinHandle<()>)> {
    let handler = Handler { reason: wide(reason), on_session_end };
    let (created_tx, created_rx) = mpsc::channel();
    let thread = std::thread::Builder::new().name(THREAD_NAME.into()).spawn(move || {
        match create_window(&handler) {
            Ok(hwnd) => {
                let _ = created_tx.send(Ok(hwnd as isize));
                run_message_loop();
            }
            Err(e) => {
                let _ = created_tx.send(Err(e));
            }
        }
        // The window is gone (or was never made): nothing references `handler` any more.
        drop(handler);
    })?;
    let hwnd = created_rx.recv().map_err(|_| io::Error::other("the session-end thread ended before creating its window"))??;
    Ok((hwnd, thread))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Registers the window class (once per process) and creates the hidden top-level window,
/// which refers to `handler` until it is destroyed: `handler` must outlive the window.
fn create_window(handler: &Handler) -> io::Result<HWND> {
    let class = wide(WINDOW_CLASS);
    // SAFETY: NULL = the module of this executable.
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    // SAFETY: plain C struct; every pointer it holds outlives the call.
    let mut wc: WNDCLASSEXW = unsafe { std::mem::zeroed() };
    wc.cbSize = size_of::<WNDCLASSEXW>() as u32;
    wc.lpfnWndProc = Some(window_proc);
    wc.hInstance = instance;
    wc.lpszClassName = class.as_ptr();
    // SAFETY: valid class description.
    if unsafe { RegisterClassExW(&wc) } == 0 && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS {
        return Err(io::Error::last_os_error());
    }
    let handler: *const Handler = handler;
    // SAFETY: registered class; not visible (no WS_VISIBLE), no parent: a top-level window. The
    // handler pointer is stored by `WM_NCCREATE` and cleared by `WM_NCDESTROY`.
    let hwnd = unsafe { CreateWindowExW(0, class.as_ptr(), class.as_ptr(), WS_OVERLAPPED, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), instance, handler.cast()) };
    if hwnd.is_null() { Err(io::Error::last_os_error()) } else { Ok(hwnd) }
}

fn run_message_loop() {
    // SAFETY: plain C struct filled by `GetMessageW`.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: valid out pointer; NULL = every window of this thread. 0 = WM_QUIT, -1 = error.
    while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        // SAFETY: message just retrieved.
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY (whole body): `hwnd` is the window being processed; `GWLP_USERDATA` holds either 0 or
    // the `Handler` stored at `WM_NCCREATE`, which outlives the window (see `spawn_window`).
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let handler = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Handler;
        if handler.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        match msg {
            WM_QUERYENDSESSION => {
                (*handler).block(hwnd, lparam);
                TRUE as LRESULT
            }
            WM_ENDSESSION => {
                (*handler).end(hwnd, wparam != 0);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_CLOSE};

    use super::*;

    /// A listener counting its callback runs; closed (thread joined) by `close`.
    struct Listener {
        hwnd: HWND,
        thread: JoinHandle<()>,
        runs: Arc<AtomicUsize>,
    }

    impl Listener {
        fn start() -> Self {
            let runs = Arc::new(AtomicUsize::new(0));
            let counter = runs.clone();
            let (hwnd, thread) = spawn_window(
                "test",
                Box::new(move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();
            Self { hwnd: hwnd as HWND, thread, runs }
        }

        fn send(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            // SAFETY: live window of the listener thread; synchronous cross-thread send.
            unsafe { SendMessageW(self.hwnd, msg, wparam, lparam) }
        }

        fn runs(&self) -> usize {
            self.runs.load(Ordering::SeqCst)
        }

        fn close(self) {
            self.send(WM_CLOSE, 0, 0);
            self.thread.join().unwrap();
        }
    }

    #[test]
    fn the_session_may_end_and_the_services_stop_when_it_does() {
        let listener = Listener::start();
        assert_eq!(listener.send(WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF as LPARAM), TRUE as LRESULT);
        assert_eq!(listener.runs(), 0, "nothing stops before the session actually ends");
        assert_eq!(listener.send(WM_ENDSESSION, 1, ENDSESSION_LOGOFF as LPARAM), 0);
        assert_eq!(listener.runs(), 1);
        listener.close();
    }

    #[test]
    fn a_cancelled_session_end_stops_nothing() {
        let listener = Listener::start();
        listener.send(WM_QUERYENDSESSION, 0, 0);
        listener.send(WM_ENDSESSION, 0, 0);
        assert_eq!(listener.runs(), 0);
        listener.close();
    }

    #[test]
    fn a_panicking_callback_does_not_take_the_listener_down() {
        let (hwnd, thread) = spawn_window("test", Box::new(|| panic!("stop failed"))).unwrap();
        // SAFETY: live window of the listener thread.
        unsafe {
            assert_eq!(SendMessageW(hwnd as HWND, WM_ENDSESSION, 1, 0), 0);
            SendMessageW(hwnd as HWND, WM_CLOSE, 0, 0);
        }
        thread.join().unwrap();
    }
}
