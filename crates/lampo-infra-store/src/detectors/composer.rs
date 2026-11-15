//! Any `composer.json` → preset `generic-php`, confidence 30; PHP from `require.php` (`minimum_php_from_constraint`).

use std::path::Path;

use lampo_domain::PortResult;
use lampo_domain::detection::DetectionHint;
use lampo_domain::ports::ProjectDetector;

use super::manifest::{Manifest, preset};

#[derive(Debug, Default, Clone, Copy)]
pub struct ComposerDetector;

impl ProjectDetector for ComposerDetector {
    fn name(&self) -> &str {
        "composer"
    }

    fn detect(&self, folder: &Path) -> PortResult<Option<DetectionHint>> {
        let manifest = Manifest::read(folder);
        if matches!(manifest, Manifest::Missing) {
            return Ok(None);
        }
        let mut evidence = vec!["found composer.json".to_string()];
        let (php, php_evidence) = manifest.php_hint();
        evidence.extend(php_evidence);
        Ok(Some(DetectionHint::new("composer", preset("generic-php"), php, 30, evidence)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_composer_json_counts_even_when_malformed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ComposerDetector.detect(dir.path()).unwrap().is_none());
        std::fs::write(dir.path().join("composer.json"), "{broken").unwrap();
        let hint = ComposerDetector.detect(dir.path()).unwrap().unwrap();
        assert_eq!((hint.confidence, hint.preset.as_str(), hint.php), (30, "generic-php", None));
    }

    #[test]
    fn php_comes_from_require_php_with_bom_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = b"\xEF\xBB\xBF".to_vec();
        bytes.extend_from_slice(br#"{"require":{"php":"^7.3|^8.0"}}"#);
        std::fs::write(dir.path().join("composer.json"), bytes).unwrap();
        assert_eq!(ComposerDetector.detect(dir.path()).unwrap().unwrap().php.unwrap().to_string(), "7.3");
    }
}
