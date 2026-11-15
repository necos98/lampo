//! `artisan` + `laravel/framework` in composer.json → preset `laravel`, confidence 90.

use std::path::Path;

use lampo_domain::PortResult;
use lampo_domain::detection::DetectionHint;
use lampo_domain::ports::ProjectDetector;

use super::manifest::{Manifest, preset};

#[derive(Debug, Default, Clone, Copy)]
pub struct LaravelDetector;

impl ProjectDetector for LaravelDetector {
    fn name(&self) -> &str {
        "laravel"
    }

    fn detect(&self, folder: &Path) -> PortResult<Option<DetectionHint>> {
        if !folder.join("artisan").is_file() {
            return Ok(None);
        }
        let manifest = Manifest::read(folder);
        if !manifest.requires("laravel/framework") {
            return Ok(None);
        }
        let (php, php_evidence) = manifest.php_hint();
        let mut evidence = vec!["found artisan".to_string(), "composer.json requires laravel/framework".to_string()];
        evidence.extend(php_evidence);
        Ok(Some(DetectionHint { project_type: "laravel".into(), preset: preset("laravel"), php, confidence: 90, evidence }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_laravel_with_php_and_rejects_lookalikes() {
        let dir = tempfile::tempdir().unwrap();
        assert!(LaravelDetector.detect(dir.path()).unwrap().is_none());
        std::fs::write(dir.path().join("artisan"), "").unwrap();
        assert!(LaravelDetector.detect(dir.path()).unwrap().is_none(), "artisan alone is not enough");
        std::fs::write(dir.path().join("composer.json"), r#"{"require":{"php":"^8.2","laravel/framework":"^11.0"}}"#).unwrap();
        let hint = LaravelDetector.detect(dir.path()).unwrap().unwrap();
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
        assert!(LaravelDetector.detect(dir.path()).unwrap().is_some());
        std::fs::write(dir.path().join("composer.json"), "{oops").unwrap();
        assert!(LaravelDetector.detect(dir.path()).unwrap().is_none());
    }
}
