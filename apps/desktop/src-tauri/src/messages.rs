//! User-facing strings shown by the Rust side (tray menu, Windows session end). Italian; keep
//! them here so i18n can be added later. The GUI's strings live in `src/lib/messages`.

pub const TRAY_TOOLTIP: &str = "Lampo";
pub const TRAY_OPEN: &str = "Apri Lampo";
pub const TRAY_CHECK_UPDATES: &str = "Controlla aggiornamenti";
pub const TRAY_QUIT: &str = "Esci";
/// Shown by Windows while Lampo holds up a logoff or shutdown to stop its services.
pub const SESSION_END_BLOCK_REASON: &str = "Arresto dei servizi di Lampo in corso…";
