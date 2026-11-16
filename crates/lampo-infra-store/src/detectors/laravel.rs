//! `artisan` + `laravel/framework` in composer.json → preset `laravel`, confidence 90.

use lampo_domain::detection::DetectionHint;

use super::manifest::{ComposerProject, preset};
use super::php::ManifestDetector;

#[derive(Debug, Default, Clone, Copy)]
pub struct LaravelDetector;

impl ManifestDetector for LaravelDetector {
    fn detect(&self, project: &ComposerProject<'_>) -> Option<DetectionHint> {
        if !project.folder().join("artisan").is_file() {
            return None;
        }
        let manifest = project.manifest();
        if !manifest.requires("laravel/framework") {
            return None;
        }
        let (php, php_evidence) = manifest.php_hint();
        let mut evidence = vec!["found artisan".to_string(), "composer.json requires laravel/framework".to_string()];
        evidence.extend(php_evidence);
        Some(DetectionHint::new("laravel", preset("laravel"), php, 90, evidence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(folder: &std::path::Path) -> Option<DetectionHint> {
        LaravelDetector.detect(&ComposerProject::new(folder))
    }

    #[test]
    fn detects_laravel_with_php_and_rejects_lookalikes() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect(dir.path()).is_none());
        std::fs::write(dir.path().join("artisan"), "").unwrap();
        assert!(detect(dir.path()).is_none(), "artisan alone is not enough");
        std::fs::write(dir.path().join("composer.json"), r#"{"require":{"php":"^8.2","laravel/framework":"^11.0"}}"#).unwrap();
        let hint = detect(dir.path()).unwrap();
        assert_eq!(hint.project_type, "laravel");
        assert_eq!(hint.preset.as_str(), "laravel");
        assert_eq!(hint.confidence, 90);
        assert_eq!(hint.php.unwrap().to_string(), "8.2");
        assert!(hint.evidence.iter().any(|e| e.contains("^8.2")));
    }

    #[test]
    fn framework_in_require_dev_and_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("artisan"), "").unwrap();
        std::fs::write(dir.path().join("composer.json"), r#"{"require-dev":{"laravel/framework":"*"}}"#).unwrap();
        assert!(detect(dir.path()).is_some());
        std::fs::write(dir.path().join("composer.json"), "{oops").unwrap();
        assert!(detect(dir.path()).is_none());
    }
}
