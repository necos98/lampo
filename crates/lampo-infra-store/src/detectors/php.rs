//! The detectors that read `composer.json`, run together so the manifest is read once per
//! detection ([`ComposerProject`]). Open-closed: add a module + list it in [`MANIFEST_DETECTORS`].

use std::path::Path;

use lampo_domain::PortResult;
use lampo_domain::detection::{DetectionHint, best_hint};
use lampo_domain::ports::ProjectDetector;

use super::composer::ComposerDetector;
use super::laravel::LaravelDetector;
use super::manifest::ComposerProject;
use super::symfony::SymfonyDetector;
use super::wordpress::WordPressDetector;

/// A framework detector working on the shared [`ComposerProject`]; `None` when the folder is
/// not this kind of project.
pub trait ManifestDetector: Send + Sync {
    fn detect(&self, project: &ComposerProject<'_>) -> Option<DetectionHint>;
}

/// In tie-break order: on equal confidence the first one wins ([`best_hint`]).
const MANIFEST_DETECTORS: [&dyn ManifestDetector; 4] = [&LaravelDetector, &WordPressDetector, &SymfonyDetector, &ComposerDetector];

/// The most confident hint of the [`MANIFEST_DETECTORS`]. Only the best typed hint survives
/// `combine_hints`, so picking it here gives the same result as reporting each one.
#[derive(Debug, Default, Clone, Copy)]
pub struct PhpDetector;

impl ProjectDetector for PhpDetector {
    fn name(&self) -> &str {
        "php"
    }

    fn detect(&self, folder: &Path) -> PortResult<Option<DetectionHint>> {
        let project = ComposerProject::new(folder);
        Ok(best_hint(MANIFEST_DETECTORS.iter().filter_map(|d| d.detect(&project)).collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(files: &[(&str, &str)]) -> Option<DetectionHint> {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            std::fs::write(dir.path().join(name), content).unwrap();
        }
        PhpDetector.detect(dir.path()).unwrap()
    }

    #[test]
    fn the_most_confident_framework_wins_and_ties_go_to_the_first() {
        let laravel = r#"{"require":{"php":"^8.2","laravel/framework":"^11"}}"#;
        let hint = detect(&[("artisan", ""), ("wp-config.php", ""), ("composer.json", laravel)]).unwrap();
        assert_eq!((hint.project_type.as_str(), hint.php.unwrap().to_string()), ("laravel", "8.2".to_string()));
        assert_eq!(detect(&[("composer.json", "{}")]).unwrap().project_type, "composer");
        assert!(detect(&[("index.php", "")]).is_none());
    }

    #[test]
    fn an_oversized_manifest_is_treated_as_unreadable() {
        let big = format!(r#"{{"require":{{"laravel/framework":"^11"}}}}{}"#, " ".repeat(crate::detectors::MAX_FILE_BYTES as usize));
        let hint = detect(&[("artisan", ""), ("composer.json", &big)]).unwrap();
        assert_eq!((hint.project_type.as_str(), hint.php), ("composer", None));
    }
}
