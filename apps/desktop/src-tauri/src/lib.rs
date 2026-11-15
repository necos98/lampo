//! Lampo desktop app: Tauri shell and composition root.
//!
//! * [`composition`] wires adapters into use cases (the only place naming adapters);
//! * [`commands`] is the Tauri command surface (thin delegation);
//! * [`events`] forwards domain events to the webview;
//! * [`tray`] owns the tray icon and the destroy-on-close window lifecycle;
//! * the control-pipe server is started here so the CLI can drive the running app.

mod commands;
mod composition;
mod events;
mod messages;
mod state;
mod tray;

use tauri::{Manager, RunEvent};

/// The CLI starts the app with this flag: tray only, no webview window.
pub const HEADLESS_FLAG: &str = "--headless";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let headless = std::env::args().any(|a| a == HEADLESS_FLAG);

    let app = tauri::Builder::default()
        // A second launch (e.g. from the Start menu) focuses the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !args.iter().any(|a| a == HEADLESS_FLAG) {
                tray::show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // In-app updates (docs/UPDATER.md): the GUI checks, downloads, calls `prepare_update`
        // (graceful service stop) and installs; `process` is the relaunch fallback.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(move |app| {
            let built = composition::build(app.handle().clone()).map_err(|e| e.message)?;
            let startup = built.state.startup.clone();
            let environment = built.state.environment.clone();
            app.manage(built.state);

            // Service restarts/stops a sync postponed while a database operation used the
            // service, applied once the operation finishes.
            tauri::async_runtime::spawn(async move { environment.apply_deferred_changes().await });

            // Control pipe for the CLI (single owner of service processes).
            let (server, handler) = (built.control_server, built.control_handler);
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server.serve(handler).await {
                    log::error!("control pipe stopped: {e}");
                }
            });

            // Bring the machine to the configured state (no UAC at boot).
            tauri::async_runtime::spawn(async move {
                if let Err(e) = startup.run().await {
                    log::error!("startup: {e}");
                }
            });

            tray::create(app.handle())?;
            if !headless {
                tray::show_main_window(app.handle());
            }
            Ok(())
        })
        .invoke_handler(lampo_command_handler!())
        .build(tauri::generate_context!())
        .expect("error while building the Lampo application");

    app.run(|_app, event| {
        // Last window closed: the webview is already destroyed; keep the core alive in the tray.
        // An explicit `app.exit(code)` (tray "Esci", `quit_app`, control `quit`) has a code.
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            api.prevent_exit();
        }
    });
}
