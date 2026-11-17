//! `mysql defender-exclusion`: what happened, and what the user can do about it.

use lampo_application::dto::{ScanExclusionChangeDto, ScanExclusionOutcomeDto, ScanExclusionResultDto};

pub fn scan_exclusion(r: &ScanExclusionResultDto) -> String {
    let folder = &r.folder;
    match (r.outcome, r.change) {
        (ScanExclusionOutcomeDto::Applied, ScanExclusionChangeDto::Add) => format!(
            "Microsoft Defender no longer scans {folder} (the data of every MySQL instance).\n\
             Undo it with `lampo mysql defender-exclusion remove --allow-elevation`."
        ),
        (ScanExclusionOutcomeDto::Applied, ScanExclusionChangeDto::Remove) => format!("Microsoft Defender scans {folder} again."),
        (ScanExclusionOutcomeDto::Cancelled, _) => "The UAC prompt was declined: nothing changed.".to_string(),
        (ScanExclusionOutcomeDto::DefenderUnavailable, _) => "Microsoft Defender is not installed or is not the active antivirus (another antivirus, or Defender turned off): \
             nothing changed. Another antivirus has its own exclusions."
            .to_string(),
        (ScanExclusionOutcomeDto::Refused, _) => "Microsoft Defender refused the change: Tamper Protection or a policy of whoever manages this PC (group policy, \
             Intune) controls its exclusions. Nothing changed."
            .to_string(),
        (ScanExclusionOutcomeDto::NotConfirmed, _) => "Microsoft Defender accepted the request but does not show the change: its exclusions are probably managed \
             or hidden by a policy (group policy, Intune). Ask whoever manages this PC."
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLDER: &str = r"C:\Users\me\AppData\Local\Lampo\mysql";

    fn result(change: ScanExclusionChangeDto, outcome: ScanExclusionOutcomeDto) -> ScanExclusionResultDto {
        ScanExclusionResultDto { change, outcome, folder: FOLDER.into() }
    }

    #[test]
    fn a_success_names_the_folder_and_how_to_undo_it() {
        let added = scan_exclusion(&result(ScanExclusionChangeDto::Add, ScanExclusionOutcomeDto::Applied));
        assert!(added.contains(FOLDER) && added.contains("defender-exclusion remove"), "{added}");
        assert_eq!(scan_exclusion(&result(ScanExclusionChangeDto::Remove, ScanExclusionOutcomeDto::Applied)), format!("Microsoft Defender scans {FOLDER} again."));
    }

    #[test]
    fn every_other_outcome_says_why_nothing_changed() {
        for (outcome, cause) in [
            (ScanExclusionOutcomeDto::Cancelled, "declined"),
            (ScanExclusionOutcomeDto::DefenderUnavailable, "not the active antivirus"),
            (ScanExclusionOutcomeDto::Refused, "Tamper Protection"),
            (ScanExclusionOutcomeDto::NotConfirmed, "policy"),
        ] {
            let text = scan_exclusion(&result(ScanExclusionChangeDto::Add, outcome));
            assert!(text.contains(cause), "{outcome:?}: {text}");
        }
    }
}
