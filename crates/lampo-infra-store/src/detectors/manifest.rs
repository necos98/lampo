//! Shared reading of a project's `composer.json` for the detectors (read-only helper, not a
//! detector itself).

use std::path::Path;

use lampo_domain::php::{PhpMinor, minimum_php_from_constraint};
use lampo_domain::preset::PresetId;

/// What a folder's `composer.json` looks like.
#[derive(Debug)]
pub enum Manifest {
    Missing,
    /// Present but not valid JSON (or unreadable).
    Unreadable,
    Parsed(serde_json::Value),
}

impl Manifest {
    pub fn read(folder: &Path) -> Self {
        match std::fs::read(folder.join("composer.json")) {
            Ok(bytes) => {
                let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
                serde_json::from_slice(bytes).map(Manifest::Parsed).unwrap_or(Manifest::Unreadable)
            }
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

pub fn preset(id: &str) -> PresetId {
    PresetId::new(id).expect("built-in preset ids are valid")
}
