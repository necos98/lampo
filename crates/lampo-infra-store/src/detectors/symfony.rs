//! `bin/console` + `symfony/framework-bundle` → preset `symfony`, confidence 85.

use std::path::Path;

use lampo_domain::PortResult;
use lampo_domain::detection::DetectionHint;
use lampo_domain::ports::ProjectDetector;

use super::manifest::{Manifest, preset};

#[derive(Debug, Default, Clone, Copy)]
pub struct SymfonyDetector;

impl ProjectDetector for SymfonyDetector {
    fn name(&self) -> &str {
        "symfony"
    }

    fn detect(&self, folder: &Path) -> PortResult<Option<DetectionHint>> {
        if !folder.join("bin").join("console").is_file() {
            return Ok(None);
        }
        let manifest = Manifest::read(folder);
        if !manifest.requires("symfony/framework-bundle") {
            return Ok(None);
        }
        let (php, php_evidence) = manifest.php_hint();
        let mut evidence = vec!["found bin/console".to_string(), "composer.json requires symfony/framework-bundle".to_string()];
        evidence.extend(php_evidence);
        Ok(Some(DetectionHint { project_type: "symfony".into(), preset: preset("symfony"), php, confidence: 85, evidence }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_symfony() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("bin")).unwrap();
        std::fs::write(dir.path().join("bin").join("console"), "").unwrap();
        assert!(SymfonyDetector.detect(dir.path()).unwrap().is_none());
        std::fs::write(dir.path().join("composer.json"), r#"{"require":{"php":">=8.1","symfony/framework-bundle":"^6.4"}}"#).unwrap();
        let hint = SymfonyDetector.detect(dir.path()).unwrap().unwrap();
        assert_eq!((hint.project_type.as_str(), hint.confidence, hint.preset.as_str()), ("symfony", 85, "symfony"));
        assert_eq!(hint.php.unwrap().to_string(), "8.1");
    }
}
