//! Any `composer.json` → preset `generic-php`, confidence 30; PHP from `require.php` (`minimum_php_from_constraint`).

use lampo_domain::detection::DetectionHint;

use super::manifest::{ComposerProject, Manifest, preset};
use super::php::ManifestDetector;

#[derive(Debug, Default, Clone, Copy)]
pub struct ComposerDetector;

impl ManifestDetector for ComposerDetector {
    fn detect(&self, project: &ComposerProject<'_>) -> Option<DetectionHint> {
        let manifest = project.manifest();
        if matches!(manifest, Manifest::Missing) {
            return None;
        }
        let mut evidence = vec!["found composer.json".to_string()];
        let (php, php_evidence) = manifest.php_hint();
        evidence.extend(php_evidence);
        Some(DetectionHint::new("composer", preset("generic-php"), php, 30, evidence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(folder: &std::path::Path) -> Option<DetectionHint> {
        ComposerDetector.detect(&ComposerProject::new(folder))
    }

    #[test]
    fn any_composer_json_counts_even_when_malformed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect(dir.path()).is_none());
        std::fs::write(dir.path().join("composer.json"), "{broken").unwrap();
        let hint = detect(dir.path()).unwrap();
        assert_eq!((hint.confidence, hint.preset.as_str(), hint.php), (30, "generic-php", None));
    }

    #[test]
    fn php_comes_from_require_php_with_bom_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = b"\xEF\xBB\xBF".to_vec();
        bytes.extend_from_slice(br#"{"require":{"php":"^7.3|^8.0"}}"#);
        std::fs::write(dir.path().join("composer.json"), bytes).unwrap();
        assert_eq!(detect(dir.path()).unwrap().php.unwrap().to_string(), "7.3");
    }
}
