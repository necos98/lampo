//! Project-type detectors, one per framework (open-closed: add a module + list it in [`all`]).

use std::sync::Arc;

use lampo_domain::ports::ProjectDetector;

pub mod composer;
pub mod laravel;
mod manifest;
pub mod symfony;
pub mod wordpress;

pub use composer::ComposerDetector;
pub use laravel::LaravelDetector;
pub use symfony::SymfonyDetector;
pub use wordpress::WordPressDetector;

/// Every built-in detector, in tie-break order.
pub fn all() -> Vec<Arc<dyn ProjectDetector>> {
    vec![Arc::new(LaravelDetector), Arc::new(WordPressDetector), Arc::new(SymfonyDetector), Arc::new(ComposerDetector)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use lampo_domain::detection::best_hint;

    fn best(files: &[(&str, &str)]) -> Option<String> {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let p = dir.path().join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        let hints = all().iter().filter_map(|d| d.detect(dir.path()).unwrap()).collect();
        best_hint(hints).map(|h| h.project_type)
    }

    #[test]
    fn framework_beats_plain_composer() {
        let laravel = r#"{"require":{"laravel/framework":"^11"}}"#;
        assert_eq!(best(&[("artisan", ""), ("composer.json", laravel)]).as_deref(), Some("laravel"));
        assert_eq!(best(&[("composer.json", "{}")]).as_deref(), Some("composer"));
        assert_eq!(best(&[("wp-config.php", "")]).as_deref(), Some("wordpress"));
        assert_eq!(best(&[("index.php", "")]), None);
    }
}
