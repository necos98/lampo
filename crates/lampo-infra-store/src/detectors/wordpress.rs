//! `wp-config.php` or `wp-load.php` → preset `wordpress`, confidence 90.

use lampo_domain::detection::DetectionHint;

use super::manifest::{ComposerProject, preset};
use super::php::ManifestDetector;

#[derive(Debug, Default, Clone, Copy)]
pub struct WordPressDetector;

impl ManifestDetector for WordPressDetector {
    fn detect(&self, project: &ComposerProject<'_>) -> Option<DetectionHint> {
        let markers: Vec<String> = ["wp-config.php", "wp-load.php"].iter().filter(|m| project.folder().join(m).is_file()).map(|m| format!("found {m}")).collect();
        if markers.is_empty() {
            return None;
        }
        let (php, php_evidence) = project.manifest().php_hint();
        let mut evidence = markers;
        evidence.extend(php_evidence);
        Some(DetectionHint::new("wordpress", preset("wordpress"), php, 90, evidence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(folder: &std::path::Path) -> Option<DetectionHint> {
        WordPressDetector.detect(&ComposerProject::new(folder))
    }

    #[test]
    fn detects_either_marker() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect(dir.path()).is_none());
        std::fs::write(dir.path().join("wp-load.php"), "").unwrap();
        let hint = detect(dir.path()).unwrap();
        assert_eq!((hint.project_type.as_str(), hint.confidence), ("wordpress", 90));
        assert_eq!(hint.evidence, ["found wp-load.php"]);
        std::fs::write(dir.path().join("wp-config.php"), "").unwrap();
        assert_eq!(detect(dir.path()).unwrap().evidence.len(), 2);
    }
}
