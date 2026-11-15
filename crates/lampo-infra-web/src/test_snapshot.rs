//! Minimal snapshot testing: rendered text is compared with `snapshots/<name>` in the crate.
//! Run with `LAMPO_UPDATE_SNAPSHOTS=1` to (re)write the files after an intended change, then
//! review the diff.

use std::path::PathBuf;

pub fn assert_snapshot(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("snapshots").join(name);
    if std::env::var_os("LAMPO_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing snapshot {}: run with LAMPO_UPDATE_SNAPSHOTS=1 to create it", path.display()));
    // Git may check the files out with CRLF line endings.
    let normalize = |s: &str| s.replace("\r\n", "\n");
    if normalize(&expected) != normalize(actual) {
        panic!("snapshot {} differs (LAMPO_UPDATE_SNAPSHOTS=1 to accept):\n--- expected\n{expected}\n--- actual\n{actual}", path.display());
    }
}
