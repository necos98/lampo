//! Shared reading of a project's `composer.json` for the detectors (read-only helper, not a
//! detector itself).

use std::cell::OnceCell;
use std::io::Read;
use std::path::Path;

use lampo_domain::php::{PhpMinor, minimum_php_from_constraint};
use lampo_domain::preset::PresetId;

use super::MAX_FILE_BYTES;

const MANIFEST_FILE: &str = "composer.json";
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// What a folder's `composer.json` looks like.
#[derive(Debug)]
pub enum Manifest {
    Missing,
    /// Present but not valid JSON, unreadable or larger than [`MAX_FILE_BYTES`].
    Unreadable,
    Parsed(serde_json::Value),
}

impl Manifest {
    pub fn read(folder: &Path) -> Self {
        match read_bounded(&folder.join(MANIFEST_FILE)) {
            Ok(Some(bytes)) => {
                let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(&bytes);
                serde_json::from_slice(bytes).map(Manifest::Parsed).unwrap_or(Manifest::Unreadable)
            }
            Ok(None) => Manifest::Unreadable,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Manifest::Missing,
            Err(_) => Manifest::Unreadable,
        }
    }

    /// Whether `package` is in `require` or `require-dev`.
    pub fn requires(&self, package: &str) -> bool {
        let Manifest::Parsed(json) = self else { return false };
        ["require", "require-dev"].iter().any(|section| json.get(section).and_then(|s| s.get(package)).is_some())
    }

    /// The raw `require.php` constraint, if any.
    pub fn php_constraint(&self) -> Option<&str> {
        let Manifest::Parsed(json) = self else { return None };
        json.get("require")?.get("php")?.as_str()
    }

    /// PHP suggested by `require.php`, with the evidence line that explains it.
    pub fn php_hint(&self) -> (Option<PhpMinor>, Option<String>) {
        match self.php_constraint() {
            Some(c) => match minimum_php_from_constraint(c) {
                Some(php) => (Some(php), Some(format!("composer.json requires php {c}"))),
                None => (None, None),
            },
            None => (None, None),
        }
    }
}

/// The file's bytes, `None` when it is larger than [`MAX_FILE_BYTES`] (never read past it).
fn read_bounded(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= MAX_FILE_BYTES).then_some(bytes))
}

/// One detection of one folder, shared by the composer-based detectors: the manifest is read
/// at most once, by the first detector that asks for it.
pub struct ComposerProject<'a> {
    folder: &'a Path,
    manifest: OnceCell<Manifest>,
}

impl<'a> ComposerProject<'a> {
    pub fn new(folder: &'a Path) -> Self {
        Self { folder, manifest: OnceCell::new() }
    }

    pub fn folder(&self) -> &Path {
        self.folder
    }

    pub fn manifest(&self) -> &Manifest {
        self.manifest.get_or_init(|| Manifest::read(self.folder))
    }
}

pub fn preset(id: &str) -> PresetId {
    PresetId::new(id).expect("built-in preset ids are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_over_the_limit_is_unreadable_and_one_at_the_limit_is_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(MANIFEST_FILE);
        let at_limit = |extra: usize| {
            let json = r#"{"require":{"php":"^8.2"}}"#;
            let padding = MAX_FILE_BYTES as usize - json.len() + extra;
            format!("{json}{}", " ".repeat(padding))
        };
        std::fs::write(&file, at_limit(0)).unwrap();
        assert_eq!(Manifest::read(dir.path()).php_constraint(), Some("^8.2"));
        std::fs::write(&file, at_limit(1)).unwrap();
        assert!(matches!(Manifest::read(dir.path()), Manifest::Unreadable));
    }

    #[test]
    fn a_missing_manifest_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(Manifest::read(dir.path()), Manifest::Missing));
    }

    #[test]
    fn the_project_reads_its_manifest_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(MANIFEST_FILE);
        std::fs::write(&file, r#"{"require":{"php":"^8.1"}}"#).unwrap();
        let project = ComposerProject::new(dir.path());
        assert_eq!(project.manifest().php_constraint(), Some("^8.1"));
        // Changing or removing the file afterwards is not seen: nothing reads it again.
        std::fs::remove_file(&file).unwrap();
        assert_eq!(project.manifest().php_constraint(), Some("^8.1"));
    }
}
