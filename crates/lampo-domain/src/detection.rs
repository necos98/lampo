//! Project-type detection results. Detectors themselves are ports
//! ([`crate::ports::ProjectDetector`]); one implementation per framework (open-closed).

use serde::{Deserialize, Serialize};

use crate::php::PhpMinor;
use crate::preset::PresetId;

/// What one detector found in a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectionHint {
    /// Short machine label (`laravel`, `wordpress`, `symfony`, `composer`).
    pub project_type: String,
    /// Preset to suggest.
    pub preset: PresetId,
    /// PHP suggested from `composer.json` (`require.php`) or similar, if any.
    pub php: Option<PhpMinor>,
    /// 0–100; the highest wins. Framework markers (`artisan`, `wp-config.php`) score higher than
    /// a bare `composer.json`.
    pub confidence: u8,
    /// Evidence, developer-facing (`found artisan`, `composer.json requires php ^8.2`).
    pub evidence: Vec<String>,
}

/// Picks the most confident hint; ties are broken by detector order (first wins).
pub fn best_hint(hints: Vec<DetectionHint>) -> Option<DetectionHint> {
    hints.into_iter().rev().max_by_key(|h| h.confidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(t: &str, c: u8) -> DetectionHint {
        DetectionHint { project_type: t.into(), preset: PresetId::new(t).unwrap(), php: None, confidence: c, evidence: vec![] }
    }

    #[test]
    fn highest_confidence_wins_and_ties_keep_order() {
        assert_eq!(best_hint(vec![hint("composer", 30), hint("laravel", 90)]).unwrap().project_type, "laravel");
        assert_eq!(best_hint(vec![hint("a", 50), hint("b", 50)]).unwrap().project_type, "a");
        assert!(best_hint(vec![]).is_none());
    }
}
