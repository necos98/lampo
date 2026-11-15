//! `wp-config.php` or `wp-load.php` → preset `wordpress`, confidence 90.

use std::path::Path;

use lampo_domain::PortResult;
use lampo_domain::detection::DetectionHint;
use lampo_domain::ports::ProjectDetector;

use super::manifest::{Manifest, preset};

#[derive(Debug, Default, Clone, Copy)]
pub struct WordPressDetector;

impl ProjectDetector for WordPressDetector {
    fn name(&self) -> &str {
        "wordpress"
    }

    fn detect(&self, folder: &Path) -> PortResult<Option<DetectionHint>> {
        let markers: Vec<String> = ["wp-config.php", "wp-load.php"].iter().filter(|m| folder.join(m).is_file()).map(|m| format!("found {m}")).collect();
        if markers.is_empty() {
            return Ok(None);
        }
        let (php, php_evidence) = Manifest::read(folder).php_hint();
        let mut evidence = markers;
        evidence.extend(php_evidence);
        Ok(Some(DetectionHint { project_type: "wordpress".into(), preset: preset("wordpress"), php, confidence: 90, evidence }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_either_marker() {
        let dir = tempfile::tempdir().unwrap();
        assert!(WordPressDetector.detect(dir.path()).unwrap().is_none());
        std::fs::write(dir.path().join("wp-load.php"), "").unwrap();
        let hint = WordPressDetector.detect(dir.path()).unwrap().unwrap();
        assert_eq!((hint.project_type.as_str(), hint.confidence), ("wordpress", 90));
        assert_eq!(hint.evidence, ["found wp-load.php"]);
        std::fs::write(dir.path().join("wp-config.php"), "").unwrap();
        assert_eq!(WordPressDetector.detect(dir.path()).unwrap().unwrap().evidence.len(), 2);
    }
}
