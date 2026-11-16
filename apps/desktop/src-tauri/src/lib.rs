//! Lampo desktop app: Tauri shell and composition root.
//!
//! * [`composition`] wires adapters into use cases (the only place naming adapters);
//! * [`commands`] is the Tauri command surface (thin delegation);
//! * [`events`] forwards domain events to the webview;
//! * [`tray`] owns the tray icon and the destroy-on-close window lifecycle;
//! * [`orderly_exit`] stops the services however the app goes away, [`session_end`] at the end
//!   of the Windows session;
//! * the control-pipe server is started here so the CLI can drive the running app.

mod commands;
mod composition;
mod events;
mod messages;
mod orderly_exit;
#[cfg(windows)]
mod session_end;
mod state;
mod tray;

use tauri::{Manager, RunEvent};

/// The CLI starts the app with this flag: tray only, no webview window.
pub const HEADLESS_FLAG: &str = "--headless";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let headless = std::env::args().any(|a| a == HEADLESS_FLAG);
    orderly_exit::install_panic_hook();

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

            // PHP error logs stay bounded during long sessions (Apache and service logs rotate
            // themselves).
            let log_rotation = built.log_rotation;
            tauri::async_runtime::spawn(async move { log_rotation.run().await });

            // Services that exit unexpectedly start again (setting `auto_restart_services`).
            let (auto_restart, crashes) = (built.auto_restart, built.crashes);
            tauri::async_runtime::spawn(auto_restart.run(crashes));

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
            #[cfg(windows)]
            {
                let handle = app.handle().clone();
                if let Err(e) = session_end::watch(messages::SESSION_END_BLOCK_REASON, move || orderly_exit::stop_services(&handle, "session end")) {
                    log::error!("cannot watch for the end of the Windows session: {e}");
                }
            }
            if !headless {
                tray::show_main_window(app.handle());
            }
            Ok(())
        })
        .invoke_handler(lampo_command_handler!())
        .build(tauri::generate_context!())
        .expect("error while building the Lampo application");

    app.run(|app, event| match event {
        // Last window closed: the webview is already destroyed; keep the core alive in the tray.
        // An explicit `app.exit(code)` (tray "Esci", `quit_app`, control `quit`) has a code.
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        // Every way out, including tao's reaction to the end of the Windows session; after a quit
        // or an update the services are already down and this only checks for MySQL leftovers.
        RunEvent::Exit => orderly_exit::stop_services(app, "app exit"),
        _ => {}
    });
}
