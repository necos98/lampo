//! Project-type detection results. Detectors themselves are ports
//! ([`crate::ports::ProjectDetector`]); one implementation per framework (open-closed).

use serde::{Deserialize, Serialize};

use crate::php::PhpMinor;
use crate::preset::PresetId;
use crate::service::ServiceKind;

/// Confidence of a hint that describes the environment a project ran in (a Docker Compose file,
/// a Dockerfile), not what kind of project it is: it never picks the type or the preset, it is
/// merged into the winning hint by [`combine_hints`].
pub const ENVIRONMENT_ONLY: u8 = 0;

/// What one detector found in a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectionHint {
    /// Short machine label (`laravel`, `wordpress`, `symfony`, `composer`, `docker`).
    pub project_type: String,
    /// Preset to suggest.
    pub preset: PresetId,
    /// PHP suggested from `composer.json` (`require.php`), a container image or similar, if any.
    pub php: Option<PhpMinor>,
    /// 0–100; the highest wins. Framework markers (`artisan`, `wp-config.php`) score higher than
    /// a bare `composer.json`; [`ENVIRONMENT_ONLY`] hints never win, they are merged.
    pub confidence: u8,
    /// Evidence, developer-facing (`found artisan`, `composer.json requires php ^8.2`).
    pub evidence: Vec<String>,
    /// Service kinds the project's own files show it uses (`mysql`, `redis`).
    #[serde(default)]
    pub services: Vec<ServiceKind>,
    /// What Lampo cannot provide for this project (PostgreSQL, search engines…), developer-facing.
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl DetectionHint {
    /// A hint without services or warnings.
    pub fn new(project_type: impl Into<String>, preset: PresetId, php: Option<PhpMinor>, confidence: u8, evidence: Vec<String>) -> Self {
        Self { project_type: project_type.into(), preset, php, confidence, evidence, services: Vec::new(), warnings: Vec::new() }
    }
}

/// Picks the most confident hint; ties are broken by detector order (first wins).
pub fn best_hint(hints: Vec<DetectionHint>) -> Option<DetectionHint> {
    hints.into_iter().rev().max_by_key(|h| h.confidence)
}

/// The detection result of a folder: the most confident typed hint ([`best_hint`]), with every
/// [`ENVIRONMENT_ONLY`] hint merged into it: the higher PHP (a container image shows the PHP
/// the project really ran on, `require.php` only its minimum), the union of services, the
/// warnings and the evidence. With environment hints only, the first one stands alone.
pub fn combine_hints(hints: Vec<DetectionHint>) -> Option<DetectionHint> {
    let (environment, typed): (Vec<_>, Vec<_>) = hints.into_iter().partition(|h| h.confidence == ENVIRONMENT_ONLY);
    let mut environment = environment.into_iter();
    let mut best = best_hint(typed).or_else(|| environment.next())?;
    for hint in environment {
        best.php = best.php.max(hint.php);
        best.evidence.extend(hint.evidence);
        best.warnings.extend(hint.warnings);
        for service in hint.services {
            if !best.services.contains(&service) {
                best.services.push(service);
            }
        }
    }
    Some(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(t: &str, c: u8) -> DetectionHint {
        DetectionHint::new(t, PresetId::new(t).unwrap(), None, c, vec![])
    }

    fn php(v: &str) -> Option<PhpMinor> {
        Some(PhpMinor::parse(v).unwrap())
    }

    #[test]
    fn highest_confidence_wins_and_ties_keep_order() {
        assert_eq!(best_hint(vec![hint("composer", 30), hint("laravel", 90)]).unwrap().project_type, "laravel");
        assert_eq!(best_hint(vec![hint("a", 50), hint("b", 50)]).unwrap().project_type, "a");
        assert!(best_hint(vec![]).is_none());
    }

    #[test]
    fn environment_hints_merge_into_the_winner() {
        let mut laravel = hint("laravel", 90);
        laravel.php = php("8.1");
        laravel.evidence = vec!["found artisan".into()];
        let mut docker = hint("docker", ENVIRONMENT_ONLY);
        docker.php = php("8.3");
        docker.evidence = vec!["compose image php:8.3-fpm".into()];
        docker.services = vec![ServiceKind::MYSQL, ServiceKind::REDIS];
        docker.warnings = vec!["PostgreSQL is not supported".into()];
        let combined = combine_hints(vec![docker.clone(), hint("composer", 30), laravel]).unwrap();
        assert_eq!(combined.project_type, "laravel");
        assert_eq!(combined.php, php("8.3"), "the image's PHP beats the require.php minimum");
        assert_eq!(combined.evidence, ["found artisan", "compose image php:8.3-fpm"]);
        assert_eq!(combined.services, [ServiceKind::MYSQL, ServiceKind::REDIS]);
        assert_eq!(combined.warnings.len(), 1);

        // A lower image PHP never lowers the composer minimum.
        let mut composer = hint("composer", 30);
        composer.php = php("8.2");
        docker.php = php("8.1");
        assert_eq!(combine_hints(vec![composer, docker.clone()]).unwrap().php, php("8.2"));

        // Alone, the environment hint is the result.
        assert_eq!(combine_hints(vec![docker]).unwrap().project_type, "docker");
        assert!(combine_hints(vec![]).is_none());
    }
}
