//! Merging the entries of several catalog sources into one catalog.
//!
//! On duplicate ids the *first* source wins for the metadata (the composition root orders the
//! sources live-first, so a URL that moved upstream stays current), but the integrity data of a
//! frozen entry always wins: `frozen.toml` pins a SHA-256 (and size) verified when the entry was
//! written, while a live hash is read from the same host that serves the binary and proves
//! nothing on its own. A live hash that differs from the pinned one is kept as a
//! [`ChecksumConflict`], and [`MergedCatalog::ensure_consistent`] refuses that id.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use super::{CatalogChannel, CatalogEntry, PackageId, Sha256Digest};
use crate::error::DomainError;

/// A live source advertises another SHA-256 than the pinned (frozen) one for the same id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksumConflict {
    pub id: PackageId,
    pub frozen: Sha256Digest,
    pub live: Sha256Digest,
}

/// The merged catalog and the integrity conflicts found while merging it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergedCatalog {
    /// One entry per id, sorted by id; a conflicting entry carries the pinned hash.
    pub entries: Vec<CatalogEntry>,
    pub conflicts: Vec<ChecksumConflict>,
}

impl MergedCatalog {
    /// `ChecksumConflict` if the sources disagree on the SHA-256 of `id`: it must not be installed.
    pub fn ensure_consistent(&self, id: &PackageId) -> Result<(), DomainError> {
        match self.conflicts.iter().find(|c| &c.id == id) {
            Some(c) => Err(DomainError::ChecksumConflict { id: c.id.to_string(), frozen: c.frozen.to_string(), live: c.live.to_string() }),
            None => Ok(()),
        }
    }
}

/// Merges entries from several sources (see the module docs for which data wins).
pub fn merge_catalogs(sources: impl IntoIterator<Item = Vec<CatalogEntry>>) -> MergedCatalog {
    let mut merged: BTreeMap<PackageId, CatalogEntry> = BTreeMap::new();
    let mut conflicts = Vec::new();
    for entry in sources.into_iter().flatten() {
        match merged.entry(entry.id.clone()) {
            Entry::Vacant(slot) => {
                slot.insert(entry);
            }
            Entry::Occupied(mut slot) => conflicts.extend(pin_integrity(slot.get_mut(), &entry)),
        }
    }
    MergedCatalog { entries: merged.into_values().collect(), conflicts }
}

/// Gives `winner` the integrity data of whichever of `winner`/`duplicate` is frozen, when the
/// other one is live; returns the conflict if both carry a hash and the hashes differ.
fn pin_integrity(winner: &mut CatalogEntry, duplicate: &CatalogEntry) -> Option<ChecksumConflict> {
    let (frozen, live) = match (winner.channel, duplicate.channel) {
        (CatalogChannel::Live, CatalogChannel::Frozen) => (duplicate, &*winner),
        (CatalogChannel::Frozen, CatalogChannel::Live) => (&*winner, duplicate),
        _ => return None,
    };
    let conflict = match (frozen.sha256, live.sha256) {
        (Some(pinned), Some(advertised)) if pinned != advertised => Some(ChecksumConflict { id: winner.id.clone(), frozen: pinned, live: advertised }),
        _ => None,
    };
    let sha256 = frozen.sha256.or(live.sha256);
    let size_bytes = frozen.size_bytes.or(live.size_bytes);
    winner.sha256 = sha256;
    winner.size_bytes = size_bytes;
    conflict
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::tests::entry;
    use crate::package::PackageKind;

    fn with(channel: CatalogChannel, hash: Option<u8>, size: Option<u64>) -> CatalogEntry {
        let mut e = entry(PackageKind::PHP, "8.3.14", None);
        e.channel = channel;
        e.sha256 = hash.map(|b| Sha256Digest::from_bytes([b; 32]));
        e.size_bytes = size;
        e
    }

    #[test]
    fn keeps_the_first_source_metadata_on_duplicates() {
        let mut live = with(CatalogChannel::Live, Some(1), None);
        live.url = crate::package::DownloadUrl::new("https://live.invalid/php.zip").unwrap();
        let frozen = with(CatalogChannel::Frozen, Some(1), Some(10));
        let merged = merge_catalogs([vec![live], vec![frozen, entry(PackageKind::PHP, "5.6.40", None)]]);
        assert_eq!(merged.entries.len(), 2);
        let php = merged.entries.iter().find(|e| e.id.version.as_str() == "8.3.14").unwrap();
        assert_eq!((php.channel, php.url.as_str()), (CatalogChannel::Live, "https://live.invalid/php.zip"));
        assert_eq!(php.size_bytes, Some(10), "the pinned size fills in the live entry");
        assert!(merged.conflicts.is_empty());
    }

    #[test]
    fn the_frozen_hash_wins_over_a_live_one() {
        for order in [[CatalogChannel::Live, CatalogChannel::Frozen], [CatalogChannel::Frozen, CatalogChannel::Live]] {
            let hash = |c: CatalogChannel| if c == CatalogChannel::Frozen { 2 } else { 1 };
            let merged = merge_catalogs(order.map(|c| vec![with(c, Some(hash(c)), None)]));
            assert_eq!(merged.entries[0].sha256, Some(Sha256Digest::from_bytes([2; 32])), "{order:?}");
            let id = &merged.entries[0].id;
            assert_eq!(merged.conflicts, [ChecksumConflict { id: id.clone(), frozen: Sha256Digest::from_bytes([2; 32]), live: Sha256Digest::from_bytes([1; 32]) }]);
            assert!(matches!(merged.ensure_consistent(id), Err(DomainError::ChecksumConflict { .. })));
        }
    }

    #[test]
    fn a_live_entry_without_a_hash_takes_the_pinned_one() {
        let merged = merge_catalogs([vec![with(CatalogChannel::Live, None, None)], vec![with(CatalogChannel::Frozen, Some(3), None)]]);
        assert_eq!(merged.entries[0].sha256, Some(Sha256Digest::from_bytes([3; 32])));
        assert!(merged.ensure_consistent(&merged.entries[0].id).is_ok());
    }

    #[test]
    fn duplicates_on_the_same_channel_keep_the_first_entry() {
        let merged = merge_catalogs([vec![with(CatalogChannel::Live, Some(1), None)], vec![with(CatalogChannel::Live, Some(2), None)]]);
        assert_eq!(merged.entries[0].sha256, Some(Sha256Digest::from_bytes([1; 32])));
        assert!(merged.conflicts.is_empty());
    }
}
