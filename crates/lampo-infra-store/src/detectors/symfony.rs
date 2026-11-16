//! `bin/console` + `symfony/framework-bundle` → preset `symfony`, confidence 85.

use lampo_domain::detection::DetectionHint;

use super::manifest::{ComposerProject, preset};
use super::php::ManifestDetector;

#[derive(Debug, Default, Clone, Copy)]
pub struct SymfonyDetector;

impl ManifestDetector for SymfonyDetector {
    fn detect(&self, project: &ComposerProject<'_>) -> Option<DetectionHint> {
        if !project.folder().join("bin").join("console").is_file() {
            return None;
        }
        let manifest = project.manifest();
        if !manifest.requires("symfony/framework-bundle") {
            return None;
        }
        let (php, php_evidence) = manifest.php_hint();
        let mut evidence = vec!["found bin/console".to_string(), "composer.json requires symfony/framework-bundle".to_string()];
        evidence.extend(php_evidence);
        Some(DetectionHint::new("symfony", preset("symfony"), php, 85, evidence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(folder: &std::path::Path) -> Option<DetectionHint> {
        SymfonyDetector.detect(&ComposerProject::new(folder))
    }

    #[test]
    fn detects_symfony() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("bin")).unwrap();
        std::fs::write(dir.path().join("bin").join("console"), "").unwrap();
        assert!(detect(dir.path()).is_none());
        std::fs::write(dir.path().join("composer.json"), r#"{"require":{"php":">=8.1","symfony/framework-bundle":"^6.4"}}"#).unwrap();
        let hint = detect(dir.path()).unwrap();
        assert_eq!((hint.project_type.as_str(), hint.confidence, hint.preset.as_str()), ("symfony", 85, "symfony"));
        assert_eq!(hint.php.unwrap().to_string(), "8.1");
    }
}
