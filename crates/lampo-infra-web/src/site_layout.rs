//! Per-site runtime folders that live next to the generated php.ini (the PHPRC dir), shared by
//! the php.ini renderer (session/upload/temp paths) and the Apache renderer (`TEMP`/`TMP`).

use std::path::{Path, PathBuf};

/// Temp files and uploads of the site (`upload_tmp_dir`, `sys_temp_dir`, `TEMP`).
pub fn temp_dir(phprc_dir: &Path) -> PathBuf {
    phprc_dir.join("tmp")
}

/// PHP file sessions (`session.save_path`), isolated per site.
pub fn session_dir(phprc_dir: &Path) -> PathBuf {
    phprc_dir.join("sessions")
}
