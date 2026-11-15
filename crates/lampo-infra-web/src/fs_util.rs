//! Small file helpers shared by the adapters.

use std::io::Write;
use std::path::Path;

use lampo_domain::{PortError, PortResult};

/// Writes `contents` to `path` atomically: temp file in the same directory, flushed, then
/// renamed over the target (readers see the old or the new file, never a partial one).
pub fn write_atomic(path: &Path, contents: &[u8]) -> PortResult<()> {
    let dir = path.parent().ok_or_else(|| PortError::io(format!("writing {}", path.display()), "path has no parent directory"))?;
    let ctx = || format!("writing {}", path.display());
    std::fs::create_dir_all(dir).map_err(|e| PortError::io(ctx(), e))?;
    let mut tmp = tempfile::Builder::new().prefix(".lampo-").suffix(".tmp").tempfile_in(dir).map_err(|e| PortError::io(ctx(), e))?;
    tmp.write_all(contents).map_err(|e| PortError::io(ctx(), e))?;
    tmp.as_file().sync_all().map_err(|e| PortError::io(ctx(), e))?;
    tmp.persist(path).map_err(|e| PortError::io(ctx(), e.error))?;
    Ok(())
}

/// `create_dir_all` with a port error.
pub fn ensure_dir(dir: &Path) -> PortResult<()> {
    std::fs::create_dir_all(dir).map_err(|e| PortError::io(format!("creating {}", dir.display()), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a").join("php.ini");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(file.parent().unwrap()).unwrap().count(), 1, "no temp file left behind");
    }
}
