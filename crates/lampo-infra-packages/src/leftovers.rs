//! The directories slot operations create next to a slot (see [`crate::slot`]) and what to do
//! with them once the operation that made them is gone.
//!
//! Each is named `<slot>.<tag>-<pid>-<nanos>`, the pid being that of the Lampo process doing the
//! operation. A live pid means the operation may still be running: its directories are left
//! alone. Of a process that is gone:
//! * a **previous** copy (`old`) is the slot's old contents, moved aside for a replacement. If
//!   the slot itself is missing, the crash hit between the two renames: the newest previous copy
//!   is the only complete package and is put back. Otherwise the replacement completed and the
//!   copy is deleted;
//! * a **staging** copy (an unfinished extraction) and a **removed** copy (`trash`, files of an
//!   uninstall or a rollback) are deleted.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::pid;

/// What a leftover directory held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Leftover {
    /// An extraction in progress.
    Staging,
    /// The slot's contents before a replacement.
    Previous,
    /// Contents on their way to deletion.
    Removed,
}

impl Leftover {
    const ALL: [Leftover; 3] = [Leftover::Staging, Leftover::Previous, Leftover::Removed];

    fn tag(self) -> &'static str {
        match self {
            Leftover::Staging => "staging",
            Leftover::Previous => "old",
            Leftover::Removed => "trash",
        }
    }
}

/// A fresh `<slot>.<tag>-<pid>-<nanos>` next to `slot`, owned by this process.
pub(crate) fn sibling(slot: &Path, kind: Leftover) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut name = slot.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".{}-{}-{nanos}", kind.tag(), std::process::id()));
    slot.with_file_name(name)
}

/// One leftover directory found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Found {
    path: PathBuf,
    kind: Leftover,
    pid: u32,
    nanos: u128,
}

/// The slot name and the leftover parts of the entry `name`, if it is a leftover.
fn parse(name: &str) -> Option<(&str, Leftover, u32, u128)> {
    Leftover::ALL.into_iter().find_map(|kind| {
        let marker = format!(".{}-", kind.tag());
        let at = name.rfind(&marker)?;
        let (pid, nanos) = name[at + marker.len()..].split_once('-')?;
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if at == 0 || !digits(pid) || !digits(nanos) {
            return None;
        }
        Some((&name[..at], kind, pid.parse().ok()?, nanos.parse().ok()?))
    })
}

/// The leftovers of `slot` (none if its directory cannot be listed).
fn leftovers_of(slot: &Path) -> Vec<Found> {
    let (Some(parent), Some(slot_name)) = (slot.parent(), slot.file_name().and_then(|n| n.to_str())) else { return Vec::new() };
    let Ok(entries) = fs::read_dir(parent) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let (owner, kind, pid, nanos) = parse(&name)?;
            (owner == slot_name && entry.file_type().is_ok_and(|t| t.is_dir())).then(|| Found { path: entry.path(), kind, pid, nanos })
        })
        .collect()
}

/// The slot directories of `dir` that have leftovers next to them.
pub(crate) fn slots_with_leftovers(dir: &Path) -> std::io::Result<BTreeSet<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(e) => return Err(e),
    };
    Ok(entries.flatten().filter_map(|entry| entry.file_name().into_string().ok().and_then(|name| parse(&name).map(|(slot, ..)| dir.join(slot)))).collect())
}

/// What to do with a missing slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingSlot {
    /// Put the newest previous copy back: the slot went missing in the middle of a replacement.
    Restore,
    /// Leave it missing: it was emptied on purpose (an uninstall).
    Leave,
}

/// Before using a slot (and at app start): restores `slot` from the newest previous copy of a
/// process that is gone if the slot is missing, then deletes the other leftovers of processes
/// that are gone (see the module docs). Best effort: a failure is logged and retried next time.
pub(crate) fn repair(slot: &Path) {
    clean(slot, MissingSlot::Restore);
}

/// After an uninstall emptied `slot`: deletes the leftovers of processes that are gone, previous
/// copies included (the package is meant to be gone).
pub(crate) fn discard(slot: &Path) {
    clean(slot, MissingSlot::Leave);
}

fn clean(slot: &Path, missing: MissingSlot) {
    let mut abandoned: Vec<Found> = leftovers_of(slot).into_iter().filter(|f| !pid::is_alive(f.pid)).collect();
    if missing == MissingSlot::Restore && !slot.exists() {
        let newest = abandoned.iter().enumerate().filter(|(_, f)| f.kind == Leftover::Previous).max_by_key(|(_, f)| f.nanos).map(|(i, _)| i);
        if let Some(previous) = newest.map(|i| abandoned.swap_remove(i)) {
            match fs::rename(&previous.path, slot) {
                Ok(()) => tracing::warn!(slot = %slot.display(), "an interrupted package update left the previous version aside; restored it"),
                Err(e) => tracing::warn!(slot = %slot.display(), from = %previous.path.display(), error = %e, "cannot restore the previous version of a package; will retry"),
            }
        }
    }
    for leftover in abandoned {
        match fs::remove_dir_all(&leftover.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(dir = %leftover.path.display(), error = %e, "cannot delete what an interrupted package operation left; will retry"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_unrelated_names_are_not_leftovers() {
        let slot = Path::new("packages").join("php-8.3");
        let name = sibling(&slot, Leftover::Previous).file_name().unwrap().to_string_lossy().into_owned();
        let (owner, kind, pid, _) = parse(&name).unwrap();
        assert_eq!((owner, kind, pid), ("php-8.3", Leftover::Previous, std::process::id()));
        assert_eq!(parse("php-redis-6.3-php8.3.trash-12-34"), Some(("php-redis-6.3-php8.3", Leftover::Removed, 12, 34)));
        for unrelated in ["php-8.3", "php-8.3.lock", "installed.toml", "php-8.3.old-", "php-8.3.old-12", "php-8.3.old-x-1", ".old-1-2", "php-8.3.staging-1-2b"] {
            assert_eq!(parse(unrelated), None, "{unrelated}");
        }
    }

    #[test]
    fn leftovers_are_found_per_slot() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["php-8.3", "php-8.3.old-1-2", "redis-8.10.staging-3-4", "redis-8.100", "php-8.3.lock"] {
            fs::create_dir_all(dir.path().join(name)).unwrap();
        }
        let slots: Vec<_> = slots_with_leftovers(dir.path()).unwrap().into_iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(slots, ["php-8.3", "redis-8.10"]);
        assert_eq!(leftovers_of(&dir.path().join("redis-8.10")).len(), 1);
        assert!(leftovers_of(&dir.path().join("redis-8.100")).is_empty(), "a longer slot name is another slot");
        assert!(slots_with_leftovers(&dir.path().join("missing")).unwrap().is_empty());
    }

    #[cfg(windows)]
    mod cleaning {
        use super::*;
        use crate::pid::exited_process_id;

        fn leftover(slot: &Path, kind: Leftover, pid: u32, nanos: u128, content: &[u8]) -> PathBuf {
            let path = slot.with_file_name(format!("{}.{}-{pid}-{nanos}", slot.file_name().unwrap().to_string_lossy(), kind.tag()));
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("php.exe"), content).unwrap();
            path
        }

        #[test]
        fn a_crash_between_the_renames_restores_the_newest_previous_version() {
            let dir = tempfile::tempdir().unwrap();
            let slot = dir.path().join("php-8.3");
            let gone = exited_process_id();
            leftover(&slot, Leftover::Previous, gone, 1, b"8.3.13");
            leftover(&slot, Leftover::Previous, gone, 2, b"8.3.14");
            leftover(&slot, Leftover::Staging, gone, 3, b"8.3.15 half extracted");
            repair(&slot);
            assert_eq!(fs::read(slot.join("php.exe")).unwrap(), b"8.3.14");
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "the other leftovers are deleted");
        }

        #[test]
        fn a_completed_replacement_only_loses_its_previous_copy() {
            let dir = tempfile::tempdir().unwrap();
            let slot = dir.path().join("php-8.3");
            fs::create_dir_all(&slot).unwrap();
            fs::write(slot.join("php.exe"), b"8.3.15").unwrap();
            leftover(&slot, Leftover::Previous, exited_process_id(), 1, b"8.3.14");
            leftover(&slot, Leftover::Removed, exited_process_id(), 2, b"8.3.12");
            repair(&slot);
            assert_eq!(fs::read(slot.join("php.exe")).unwrap(), b"8.3.15");
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }

        #[test]
        fn leftovers_of_a_live_process_are_kept() {
            let dir = tempfile::tempdir().unwrap();
            let slot = dir.path().join("php-8.3");
            let staging = leftover(&slot, Leftover::Staging, std::process::id(), 1, b"in progress");
            let previous = leftover(&slot, Leftover::Previous, std::process::id(), 2, b"8.3.14");
            repair(&slot);
            assert!(staging.exists() && previous.exists(), "another operation of a running Lampo owns them");
            assert!(!slot.exists(), "not restored while its owner may still be replacing the slot");
        }
    }
}
