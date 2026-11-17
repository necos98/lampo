//! Tray icon and window lifecycle.
//!
//! * Closing the window destroys the webview (frees its RAM); the core keeps running in the tray
//!   (`RunEvent::ExitRequested` without an exit code is prevented in `lib.rs`).
//! * "Apri Lampo" (or a tray click) recreates the window from the config.
//! * "Controlla aggiornamenti" opens the window on the settings page (updates section).
//! * "Esci" stops every service and exits.
//! * `--headless` (used by the CLI) starts with no window at all.
//! * The glyph is the orange bolt while at least one service is running, grey otherwise; it
//!   follows the `service-status` events (Rust-side listener on the same channel as the webview).

mod glyph;

use std::sync::Mutex;

use glyph::Glyph;
use lampo_application::events::{names, ServiceStatusPayload};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Listener, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::messages;
use crate::state::AppState;

pub const MAIN_WINDOW: &str = "main";
const TRAY_ID: &str = "lampo-tray";
/// SvelteKit route of the settings page (updates section).
const SETTINGS_ROUTE: &str = "/settings";

const ICON_ON: &[u8] = include_bytes!("../icons/tray/tray-on.png");
const ICON_OFF: &[u8] = include_bytes!("../icons/tray/tray-off.png");

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", messages::TRAY_OPEN, true, None::<&str>)?;
    let updates = MenuItem::with_id(app, "updates", messages::TRAY_CHECK_UPDATES, true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", messages::TRAY_QUIT, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &updates, &quit_item])?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(tray_icon(false)?)
        .tooltip(messages::TRAY_TOOLTIP)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "updates" => show_route(app, SETTINGS_ROUTE),
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    track_service_status(app);
    Ok(())
}

fn tray_icon(running: bool) -> tauri::Result<Image<'static>> {
    Image::from_bytes(if running { ICON_ON } else { ICON_OFF })
}

/// The tray's [`Glyph`], shared by the status listener and [`redraw`].
struct TrayGlyph(Mutex<Glyph>);

/// Keeps the last known state of every service and swaps the glyph when "any running" flips.
fn track_service_status(app: &AppHandle) {
    let mut glyph = Glyph::default();
    if let Some(state) = app.try_state::<AppState>() {
        match state.service_queries.list() {
            Ok(services) => services.into_iter().for_each(|s| glyph.record(s.service, s.state)),
            Err(e) => log::debug!("tray: initial service status unavailable: {e:?}"),
        }
    }
    app.manage(TrayGlyph(Mutex::new(glyph)));
    redraw(app);
    let shutdown_gate = app.try_state::<AppState>().map(|state| state.shutdown.gate());

    let handle = app.clone();
    app.listen_any(names::SERVICE_STATUS, move |event| {
        let Ok(payload) = serde_json::from_str::<ServiceStatusPayload>(event.payload()) else {
            return;
        };
        let Some(tray) = handle.try_state::<TrayGlyph>() else { return };
        tray.0.lock().unwrap_or_else(|p| p.into_inner()).record(payload.service, payload.state);
        // While the app goes away the main thread may be blocked waiting for the services to
        // stop (`orderly_exit`): an icon change, run on the main thread, would never return. The
        // state is recorded anyway, for the redraw once the gate reopens (a cancelled update).
        if !shutdown_gate.as_ref().is_some_and(|gate| gate.is_closed()) {
            redraw(&handle);
        }
    });
}

/// Draws the glyph the recorded service states call for, unless it is on screen already. Called
/// on every status change and after the shutdown gate reopens; never while it is closed.
pub fn redraw(app: &AppHandle) {
    let Some(tray) = app.try_state::<TrayGlyph>() else { return };
    // Drawn under the lock: concurrent redraws cannot leave an older glyph on screen.
    let mut glyph = tray.0.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(running) = glyph.redraw() {
        set_running(app, running);
    }
}

fn set_running(app: &AppHandle, running: bool) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    match tray_icon(running) {
        Ok(icon) => {
            if let Err(e) = tray.set_icon(Some(icon)) {
                log::warn!("tray icon update failed: {e}");
            }
        }
        Err(e) => log::warn!("tray icon decode failed: {e}"),
    }
}

/// Raw `HWND` of the main window while it is shown, the owner of the UAC prompts the app
/// raises. `None` when it is hidden or destroyed (closed to the tray).
#[cfg(windows)]
pub fn main_window_handle(app: &AppHandle) -> Option<isize> {
    let window = app.get_webview_window(MAIN_WINDOW)?;
    if !window.is_visible().unwrap_or(false) {
        return None;
    }
    window.hwnd().ok().map(|hwnd| hwnd.0 as isize)
}

#[cfg(not(windows))]
pub fn main_window_handle(_app: &AppHandle) -> Option<isize> {
    None
}

/// Shows the main window, recreating it from `tauri.conf.json` if it was destroyed.
pub fn show_main_window(app: &AppHandle) {
    open_main_window(app, None);
}

/// Shows the main window on `route`: a live window is told to navigate (the layout listens for
/// the `lampo:navigate` DOM event and uses the SvelteKit router); a destroyed one is recreated
/// directly on that route (the asset server falls back to `index.html`).
fn show_route(app: &AppHandle, route: &str) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let script = format!("window.dispatchEvent(new CustomEvent('lampo:navigate', {{ detail: {route:?} }}))");
        if let Err(e) = window.eval(script) {
            log::warn!("cannot navigate the main window to {route}: {e}");
        }
    }
    open_main_window(app, Some(route));
}

fn open_main_window(app: &AppHandle, route: Option<&str>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let Some(mut config) = app.config().app.windows.iter().find(|w| w.label == MAIN_WINDOW).cloned() else {
        log::error!("no `{MAIN_WINDOW}` window in tauri.conf.json");
        return;
    };
    if let Some(route) = route {
        config.url = WebviewUrl::App(route.trim_start_matches('/').into());
    }
    match WebviewWindowBuilder::from_config(app, &config).and_then(|b| b.build()) {
        Ok(window) => {
            let _ = window.set_focus();
        }
        Err(e) => log::error!("cannot create the main window: {e}"),
    }
}

/// Stops services gracefully, then exits the process.
pub fn quit(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(state) = app.try_state::<AppState>() {
            if let Err(e) = state.shutdown.run().await {
                log::error!("shutdown: {e}");
            }
        }
        app.exit(0);
    });
}
