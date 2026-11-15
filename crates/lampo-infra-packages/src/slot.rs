//! All-or-nothing replacement of a package slot directory (see [`crate::registry`]).
//!
//! New contents are prepared in a staging directory next to the slot (`<slot>.staging-<n>`, same
//! volume), then swapped in by [`replace_dir`]: the old slot is renamed aside (which fails
//! cleanly if a running process holds files in it), the staging directory is renamed into place
//! (restoring the old slot if that fails) and the old one deleted. The slot is therefore either
//! the complete old package or the complete new one. Leftovers of a crash are removed the next
//! time the same slot is replaced.

use std::fs;
use std::path::{Path, PathBuf};

use lampo_domain::progress::CancelFlag;
use lampo_domain::{PortError, PortResult};

use crate::entry_name::safe_file_name;

/// Moves the verified single-file payload `file` into a fresh `destination` slot as
/// `file_name`, replacing the previous slot only once the new one is complete. `file` must be on
/// the same volume as `destination` (the downloads and packages directories share the data root).
pub fn place_file_blocking(file: &Path, destination: &Path, file_name: &str, cancel: &CancelFlag) -> PortResult<()> {
    let name = safe_file_name(file_name).ok_or_else(|| PortError::invalid_data(file.display().to_string(), format!("unsafe package file name `{file_name}`")))?;
    let staging = prepare_staging(destination)?;
    let result = fs::rename(file, staging.join(&name)).map_err(|e| PortError::io(format!("moving {} into {}", file.display(), staging.display()), e)).and_then(|()| {
        cancel.check()?;
        replace_dir(&staging, destination)
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// Creates the parent of `destination`, removes leftovers of interrupted runs and returns a new,
/// empty staging directory next to it.
pub(crate) fn prepare_staging(destination: &Path) -> PortResult<PathBuf> {
    let parent = destination.parent().filter(|p| !p.as_os_str().is_empty()).ok_or_else(|| PortError::io("installing", format!("{} has no parent directory", destination.display())))?;
    fs::create_dir_all(parent).map_err(|e| PortError::io(format!("creating {}", parent.display()), e))?;
    clean_leftovers(destination);
    let staging = unique_sibling(destination, "staging");
    fs::create_dir_all(&staging).map_err(|e| PortError::io(format!("creating {}", staging.display()), e))?;
    Ok(staging)
}

/// Atomically (per rename) replaces `destination` with `staging`.
pub(crate) fn replace_dir(staging: &Path, destination: &Path) -> PortResult<()> {
    let old = if destination.exists() {
        let old = unique_sibling(destination, "old");
        fs::rename(destination, &old).map_err(|e| PortError::io(format!("replacing {} (is a service using it? stop it and retry)", destination.display()), e))?;
        Some(old)
    } else {
        None
    };
    if let Err(e) = fs::rename(staging, destination) {
        if let Some(old) = &old {
            let _ = fs::rename(old, destination);
        }
        return Err(PortError::io(format!("moving files into {}", destination.display()), e));
    }
    if let Some(old) = old {
        if let Err(e) = fs::remove_dir_all(&old) {
            tracing::warn!(dir = %old.display(), error = %e, "could not delete the previous version; it will be removed later");
        }
    }
    Ok(())
}

/// `<dest>.<tag>-<pid>-<nanos>`
pub(crate) fn unique_sibling(path: &Path, tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".{tag}-{}-{nanos}", std::process::id()));
    path.with_file_name(name)
}

/// Deletes `<dest>.staging-*` / `<dest>.old-*` / `<dest>.trash-*` left by an interrupted run.
pub(crate) fn clean_leftovers(destination: &Path) {
    let (Some(parent), Some(name)) = (destination.parent(), destination.file_name()) else { return };
    let name = name.to_string_lossy().to_string();
    let Ok(entries) = fs::read_dir(parent) else { return };
    for entry in entries.flatten() {
        let n = entry.file_name().to_string_lossy().to_string();
        let leftover = ["staging", "old", "trash"].iter().any(|tag| n.starts_with(&format!("{name}.{tag}-")));
        if leftover {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        names.sort();
        names
    }

    #[test]
    fn places_a_file_into_a_new_slot() {
        let dir = tempfile::tempdir().unwrap();
        let download = dir.path().join("downloads").join("composer-2.8.5-composer.phar");
        fs::create_dir_all(download.parent().unwrap()).unwrap();
        fs::write(&download, b"2.8.5").unwrap();
        let slot = dir.path().join("packages").join("composer-2.8");
        place_file_blocking(&download, &slot, "composer.phar", &CancelFlag::new()).unwrap();
        assert_eq!(fs::read(slot.join("composer.phar")).unwrap(), b"2.8.5");
        assert!(!download.exists(), "moved, not copied");
        assert_eq!(names(&dir.path().join("packages")), ["composer-2.8"], "no staging leftovers");
    }

    #[test]
    fn replaces_the_previous_version_of_the_slot() {
        let dir = tempfile::tempdir().unwrap();
        let slot = dir.path().join("composer-2.8");
        fs::create_dir_all(&slot).unwrap();
        fs::write(slot.join("composer.phar"), b"2.8.4").unwrap();
        fs::write(slot.join("composer.bat"), b"old wrapper").unwrap();
        let download = dir.path().join("new.phar");
        fs::write(&download, b"2.8.5").unwrap();
        place_file_blocking(&download, &slot, "composer.phar", &CancelFlag::new()).unwrap();
        assert_eq!(names(&slot), ["composer.phar"], "the slot holds exactly the new package");
        assert_eq!(fs::read(slot.join("composer.phar")).unwrap(), b"2.8.5");
    }

    #[test]
    fn a_failed_placement_keeps_the_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        let slot = dir.path().join("composer-2.8");
        fs::create_dir_all(&slot).unwrap();
        fs::write(slot.join("composer.phar"), b"2.8.4").unwrap();
        let missing = dir.path().join("never-downloaded.phar");
        assert!(matches!(place_file_blocking(&missing, &slot, "composer.phar", &CancelFlag::new()), Err(PortError::Io { .. })));
        let cancel = CancelFlag::new();
        cancel.cancel();
        let download = dir.path().join("new.phar");
        fs::write(&download, b"2.8.5").unwrap();
        assert_eq!(place_file_blocking(&download, &slot, "composer.phar", &cancel), Err(PortError::Cancelled));
        assert_eq!(fs::read(slot.join("composer.phar")).unwrap(), b"2.8.4");
        assert_eq!(names(dir.path()), ["composer-2.8"], "staging removed");
    }

    #[test]
    fn unsafe_file_names_are_refused_before_touching_the_slot() {
        let dir = tempfile::tempdir().unwrap();
        let download = dir.path().join("new.phar");
        fs::write(&download, b"x").unwrap();
        for evil in ["..\\evil.phar", "sub/composer.phar", "CON", "composer.phar."] {
            let err = place_file_blocking(&download, &dir.path().join("slot"), evil, &CancelFlag::new()).unwrap_err();
            assert!(matches!(err, PortError::InvalidData { .. }), "{evil}: {err:?}");
        }
        assert_eq!(names(dir.path()), ["new.phar"]);
    }

    #[test]
    fn leftovers_of_a_crash_are_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("redis-8.10");
        fs::create_dir_all(dir.path().join("redis-8.10.staging-1-2")).unwrap();
        fs::create_dir_all(dir.path().join("redis-8.10.old-1-2")).unwrap();
        fs::create_dir_all(dir.path().join("redis-8.100")).unwrap();
        let staging = prepare_staging(&dest).unwrap();
        replace_dir(&staging, &dest).unwrap();
        assert!(!dir.path().join("redis-8.10.staging-1-2").exists());
        assert!(!dir.path().join("redis-8.10.old-1-2").exists());
        assert!(dir.path().join("redis-8.100").exists(), "unrelated directories are untouched");
    }
}
