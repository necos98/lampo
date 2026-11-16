//! Project-type detectors, one per framework (open-closed: add a module + list it in [`all`],
//! or in `php::MANIFEST_DETECTORS` when it reads `composer.json`).

use std::sync::Arc;

use lampo_domain::ports::ProjectDetector;

mod composer;
pub mod docker;
mod laravel;
mod manifest;
mod php;
mod symfony;
mod wordpress;

pub use docker::DockerDetector;
pub use php::PhpDetector;

/// Files larger than this are not configuration: skipped (`composer.json` is then unreadable).
const MAX_FILE_BYTES: u64 = 256 * 1024;

/// Every built-in detector, in tie-break order (environment-only ones, Docker, merge into the
/// winner: `lampo_domain::detection::combine_hints`).
pub fn all() -> Vec<Arc<dyn ProjectDetector>> {
    vec![Arc::new(PhpDetector), Arc::new(DockerDetector)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use lampo_domain::detection::{best_hint, combine_hints};

    fn hints(files: &[(&str, &str)]) -> Vec<lampo_domain::detection::DetectionHint> {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let p = dir.path().join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        all().iter().filter_map(|d| d.detect(dir.path()).unwrap()).collect()
    }

    fn best(files: &[(&str, &str)]) -> Option<String> {
        best_hint(hints(files)).map(|h| h.project_type)
    }

    #[test]
    fn framework_beats_plain_composer() {
        let laravel = r#"{"require":{"laravel/framework":"^11"}}"#;
        assert_eq!(best(&[("artisan", ""), ("composer.json", laravel)]).as_deref(), Some("laravel"));
        assert_eq!(best(&[("composer.json", "{}")]).as_deref(), Some("composer"));
        assert_eq!(best(&[("wp-config.php", "")]).as_deref(), Some("wordpress"));
        assert_eq!(best(&[("index.php", "")]), None);
    }

    #[test]
    fn a_sail_project_is_laravel_with_the_runtime_php_and_redis() {
        let laravel = r#"{"require":{"php":"^8.2","laravel/framework":"^12"}}"#;
        let compose = "services:\n  laravel.test:\n    build:\n      context: ./vendor/laravel/sail/runtimes/8.4\n  redis:\n    image: redis:alpine\n";
        let hint = combine_hints(hints(&[("artisan", ""), ("composer.json", laravel), ("compose.yaml", compose)])).unwrap();
        assert_eq!((hint.project_type.as_str(), hint.preset.as_str()), ("laravel", "laravel"));
        assert_eq!(hint.php.unwrap().to_string(), "8.4");
        assert_eq!(hint.services.iter().map(|s| s.as_str()).collect::<Vec<_>>(), ["redis"]);
        assert!(hint.evidence.contains(&"found artisan".to_string()) && hint.evidence.contains(&"found compose.yaml".to_string()));
    }
}
