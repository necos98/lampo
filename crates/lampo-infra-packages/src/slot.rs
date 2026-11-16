//! All-or-nothing replacement of a package slot directory (see [`crate::registry`]).
//!
//! New contents are prepared in a staging directory next to the slot (`<slot>.staging-*`, same
//! volume), then swapped in by [`replace_dir`]: the old slot is renamed aside (`<slot>.old-*`,
//! which fails cleanly if a running process holds files in it) and the staging directory is
//! renamed into place (restoring the old slot if that fails). The old contents stay aside until
//! the install is complete: [`commit`] deletes them, [`roll_back`] puts them back. The slot is
//! therefore either the complete old package or the complete new one, and a failed post-install
//! step never costs the previous version. What a crash leaves is handled by
//! `leftovers` the next time the slot is replaced, and at app start.

use std::fs;
use std::path::{Path, PathBuf};

use lampo_domain::ports::SlotReplacement;
use lampo_domain::progress::CancelFlag;
use lampo_domain::{PortError, PortResult};
use lampo_infra_fs::ensure_dir;

use crate::entry_name::safe_file_name;
use crate::leftovers::{self, Leftover};

/// Moves the verified single-file payload `file` into a fresh `destination` slot as
/// `file_name`, replacing the previous slot only once the new one is complete. `file` must be on
/// the same volume as `destination` (the downloads and packages directories share the data root).
pub fn place_file_blocking(file: &Path, destination: &Path, file_name: &str, cancel: &CancelFlag) -> PortResult<SlotReplacement> {
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

/// Creates the parent of `destination`, handles leftovers of interrupted runs and returns a new,
/// empty staging directory next to it.
pub(crate) fn prepare_staging(destination: &Path) -> PortResult<PathBuf> {
    let parent = destination.parent().filter(|p| !p.as_os_str().is_empty()).ok_or_else(|| PortError::io("installing", format!("{} has no parent directory", destination.display())))?;
    ensure_dir(parent)?;
    leftovers::repair(destination);
    let staging = leftovers::sibling(destination, Leftover::Staging);
    ensure_dir(&staging)?;
    Ok(staging)
}

/// Atomically (per rename) replaces `destination` with `staging`, keeping the previous contents
/// aside until [`commit`] or [`roll_back`].
pub(crate) fn replace_dir(staging: &Path, destination: &Path) -> PortResult<SlotReplacement> {
    let previous = if destination.exists() {
        let previous = leftovers::sibling(destination, Leftover::Previous);
        fs::rename(destination, &previous).map_err(|e| PortError::io(format!("replacing {} (is a service using it? stop it and retry)", destination.display()), e))?;
        Some(previous)
    } else {
        None
    };
    if let Err(e) = fs::rename(staging, destination) {
        if let Some(previous) = &previous {
            if let Err(restore) = fs::rename(previous, destination) {
                tracing::error!(slot = %destination.display(), error = %restore, "cannot put the previous version back; it will be restored at the next start");
            }
        }
        return Err(PortError::io(format!("moving files into {}", destination.display()), e));
    }
    Ok(SlotReplacement { destination: destination.to_path_buf(), previous })
}

/// Deletes the previous contents of a completed replacement.
pub(crate) fn commit(replacement: &SlotReplacement) {
    if let Some(previous) = &replacement.previous {
        remove_or_defer(previous, "could not delete the previous version; it will be removed later");
    }
}

/// Undoes a replacement: the new contents are removed and the previous ones (if any) renamed
/// back into the slot.
pub(crate) fn roll_back(replacement: &SlotReplacement) -> PortResult<()> {
    let destination = &replacement.destination;
    let removed = leftovers::sibling(destination, Leftover::Removed);
    match fs::rename(destination, &removed) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(PortError::io(format!("removing the new files of {} (is a service using it? stop it and retry)", destination.display()), e)),
    }
    if let Some(previous) = &replacement.previous {
        // Kept aside on failure: the leftover sweep restores it into the empty slot.
        fs::rename(previous, destination).map_err(|e| PortError::io(format!("putting the previous version back into {}", destination.display()), e))?;
    }
    remove_or_defer(&removed, "could not delete the files of a failed install; they will be removed later");
    Ok(())
}

/// Deletes the directory `dir`; a failure (a file still held) leaves it to the leftover sweep.
fn remove_or_defer(dir: &Path, message: &str) {
    match fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(dir = %dir.display(), error = %e, "{message}"),
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

    /// A slot holding `composer.phar` = `version`.
    fn slot_with(dir: &Path, version: &[u8]) -> PathBuf {
        let slot = dir.join("composer-2.8");
        fs::create_dir_all(&slot).unwrap();
        fs::write(slot.join("composer.phar"), version).unwrap();
        slot
    }

    fn download(dir: &Path, version: &[u8]) -> PathBuf {
        let file = dir.join("new.phar");
        fs::write(&file, version).unwrap();
        file
    }

    #[test]
    fn places_a_file_into_a_new_slot() {
        let dir = tempfile::tempdir().unwrap();
        let download = dir.path().join("downloads").join("composer-2.8.5-composer.phar");
        fs::create_dir_all(download.parent().unwrap()).unwrap();
        fs::write(&download, b"2.8.5").unwrap();
        let slot = dir.path().join("packages").join("composer-2.8");
        let replacement = place_file_blocking(&download, &slot, "composer.phar", &CancelFlag::new()).unwrap();
        assert_eq!(replacement, SlotReplacement { destination: slot.clone(), previous: None });
        commit(&replacement);
        assert_eq!(fs::read(slot.join("composer.phar")).unwrap(), b"2.8.5");
        assert!(!download.exists(), "moved, not copied");
        assert_eq!(names(&dir.path().join("packages")), ["composer-2.8"], "no staging leftovers");
    }

    #[test]
    fn the_previous_version_stays_aside_until_the_replacement_is_committed() {
        let dir = tempfile::tempdir().unwrap();
        let slot = slot_with(dir.path(), b"2.8.4");
        fs::write(slot.join("composer.bat"), b"old wrapper").unwrap();
        let replacement = place_file_blocking(&download(dir.path(), b"2.8.5"), &slot, "composer.phar", &CancelFlag::new()).unwrap();
        assert_eq!(names(&slot), ["composer.phar"], "the slot holds exactly the new package");
        assert_eq!(fs::read(slot.join("composer.phar")).unwrap(), b"2.8.5");
        let previous = replacement.previous.clone().unwrap();
        assert_eq!(fs::read(previous.join("composer.phar")).unwrap(), b"2.8.4", "kept until committed");
        commit(&replacement);
        assert_eq!(names(dir.path()), ["composer-2.8"]);
    }

    #[test]
    fn a_rollback_puts_the_previous_version_back() {
        let dir = tempfile::tempdir().unwrap();
        let slot = slot_with(dir.path(), b"2.8.4");
        let replacement = place_file_blocking(&download(dir.path(), b"2.8.5"), &slot, "composer.phar", &CancelFlag::new()).unwrap();
        roll_back(&replacement).unwrap();
        assert_eq!(fs::read(slot.join("composer.phar")).unwrap(), b"2.8.4");
        assert_eq!(names(dir.path()), ["composer-2.8"], "the new files are deleted");
    }

    #[test]
    fn a_rollback_of_a_first_install_empties_the_slot() {
        let dir = tempfile::tempdir().unwrap();
        let slot = dir.path().join("composer-2.8");
        let replacement = place_file_blocking(&download(dir.path(), b"2.8.5"), &slot, "composer.phar", &CancelFlag::new()).unwrap();
        roll_back(&replacement).unwrap();
        assert!(names(dir.path()).is_empty(), "{:?}", names(dir.path()));
    }

    #[test]
    fn a_failed_placement_keeps_the_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        let slot = slot_with(dir.path(), b"2.8.4");
        let missing = dir.path().join("never-downloaded.phar");
        assert!(matches!(place_file_blocking(&missing, &slot, "composer.phar", &CancelFlag::new()), Err(PortError::Io { .. })));
        let cancel = CancelFlag::new();
        cancel.cancel();
        let download = download(dir.path(), b"2.8.5");
        assert_eq!(place_file_blocking(&download, &slot, "composer.phar", &cancel), Err(PortError::Cancelled));
        assert_eq!(fs::read(slot.join("composer.phar")).unwrap(), b"2.8.4");
        assert_eq!(names(dir.path()), ["composer-2.8"], "staging removed");
    }

    #[test]
    fn unsafe_file_names_are_refused_before_touching_the_slot() {
        let dir = tempfile::tempdir().unwrap();
        let download = download(dir.path(), b"x");
        for evil in ["..\\evil.phar", "sub/composer.phar", "CON", "composer.phar."] {
            let err = place_file_blocking(&download, &dir.path().join("slot"), evil, &CancelFlag::new()).unwrap_err();
            assert!(matches!(err, PortError::InvalidData { .. }), "{evil}: {err:?}");
        }
        assert_eq!(names(dir.path()), ["new.phar"]);
    }

    #[cfg(windows)]
    #[test]
    fn a_crash_between_the_renames_is_repaired_by_the_next_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("php-8.3");
        let gone = crate::pid::exited_process_id();
        let old = dir.path().join(format!("php-8.3.old-{gone}-1"));
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("php.exe"), b"8.3.14").unwrap();
        fs::create_dir_all(dir.path().join(format!("php-8.3.staging-{gone}-2"))).unwrap();
        fs::create_dir_all(dir.path().join("php-8.30")).unwrap();
        let staging = prepare_staging(&dest).unwrap();
        assert_eq!(fs::read(dest.join("php.exe")).unwrap(), b"8.3.14", "the only complete copy is back in the slot");
        let replacement = replace_dir(&staging, &dest).unwrap();
        commit(&replacement);
        assert_eq!(names(dir.path()), ["php-8.3", "php-8.30"], "leftovers gone, unrelated directories untouched");
    }

    #[test]
    fn leftovers_of_a_live_process_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("php-8.3");
        let busy = dir.path().join(format!("php-8.3.staging-{}-1", std::process::id()));
        fs::create_dir_all(&busy).unwrap();
        let staging = prepare_staging(&dest).unwrap();
        commit(&replace_dir(&staging, &dest).unwrap());
        assert!(busy.exists(), "another operation of a running Lampo may still be using it");
    }
}
