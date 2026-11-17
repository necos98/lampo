//! `defender exclusion` and `mysql defender-exclusion`: what happened, and what the user can do
//! about it.

use lampo_application::dto::{DefenderExclusionCommandDto, DefenderExclusionResultDto, ScanExclusionChangeDto, ScanExclusionOutcomeDto, ScanExclusionResultDto};

/// `mysql defender-exclusion`.
pub fn scan_exclusion(r: &ScanExclusionResultDto) -> String {
    if let Some(problem) = problem(r.outcome) {
        return problem.to_string();
    }
    let folder = &r.folder;
    match r.change {
        ScanExclusionChangeDto::Add => format!(
            "Microsoft Defender no longer scans {folder} (the data of every MySQL instance).\n\
             Undo it with `lampo mysql defender-exclusion remove --allow-elevation`."
        ),
        ScanExclusionChangeDto::Remove => format!("Microsoft Defender scans {folder} again."),
    }
}

/// `defender exclusion`: every target, one per line.
pub fn defender_exclusion(r: &DefenderExclusionResultDto) -> String {
    if let Some(problem) = problem(r.outcome) {
        return match (r.change, r.outcome) {
            (ScanExclusionChangeDto::Add, ScanExclusionOutcomeDto::Refused | ScanExclusionOutcomeDto::NotConfirmed) => {
                format!("{problem}\nSend the output of `lampo defender exclusion commands {}` to whoever manages this PC.", selectors(r))
            }
            _ => problem.to_string(),
        };
    }
    let mut lines = vec![match r.change {
        ScanExclusionChangeDto::Add => "Microsoft Defender no longer scans:".to_string(),
        ScanExclusionChangeDto::Remove => "Microsoft Defender scans again:".to_string(),
    }];
    lines.extend(r.folder.iter().map(|folder| format!("  {folder} (the data of every MySQL instance)")));
    lines.extend(r.processes.iter().map(|exe| format!("  the files opened by {exe}")));
    lines.extend(r.projects.iter().map(|folder| format!("  {folder} (your projects, with their vendor and node_modules folders)")));
    if r.change == ScanExclusionChangeDto::Add {
        lines.push(format!("Undo it with `lampo defender exclusion remove {} --allow-elevation`.", selectors(r)));
        if !r.processes.is_empty() {
            lines.push("After installing another PHP version, run `lampo defender exclusion add --php --allow-elevation` again.".to_string());
        }
    }
    lines.join("\n")
}

/// The selectors naming the targets of `r` again (`--php --mysql --projects "<folder>"`).
fn selectors(r: &DefenderExclusionResultDto) -> String {
    let projects = r.projects.as_ref().map(|folder| format!("--projects \"{folder}\""));
    let selectors: Vec<&str> = [(!r.processes.is_empty()).then_some("--php"), r.folder.is_some().then_some("--mysql"), projects.as_deref()].into_iter().flatten().collect();
    selectors.join(" ")
}

/// `defender exclusion commands`: the PowerShell lines only, one per exclusion, ready to be sent
/// or pasted.
pub fn defender_commands(commands: &[DefenderExclusionCommandDto]) -> String {
    commands.iter().map(|c| c.command.as_str()).collect::<Vec<_>>().join("\n")
}

/// Why the request did not apply; `None` when it did.
fn problem(outcome: ScanExclusionOutcomeDto) -> Option<&'static str> {
    match outcome {
        ScanExclusionOutcomeDto::Applied => None,
        ScanExclusionOutcomeDto::Cancelled => Some("The UAC prompt was declined: nothing changed."),
        ScanExclusionOutcomeDto::DefenderUnavailable => Some(
            "Microsoft Defender is not installed or is not the active antivirus (another antivirus, or Defender turned off): \
             nothing changed. Another antivirus has its own exclusions.",
        ),
        ScanExclusionOutcomeDto::Refused => Some(
            "Microsoft Defender refused the change: Tamper Protection or a policy of whoever manages this PC (group policy, \
             Intune) controls its exclusions.",
        ),
        ScanExclusionOutcomeDto::NotConfirmed => Some(
            "Microsoft Defender accepted the request but does not show the change: its exclusions are probably managed \
             or hidden by a policy (group policy, Intune). Ask whoever manages this PC.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLDER: &str = r"C:\Users\me\AppData\Local\Lampo\mysql";
    const PHP: &str = r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe";
    const HTTPD: &str = r"C:\Users\me\AppData\Local\Lampo\packages\apache-2.4\bin\httpd.exe";

    fn result(change: ScanExclusionChangeDto, outcome: ScanExclusionOutcomeDto) -> ScanExclusionResultDto {
        ScanExclusionResultDto { change, outcome, folder: FOLDER.into() }
    }

    fn defender(change: ScanExclusionChangeDto, folder: Option<&str>, processes: &[&str]) -> DefenderExclusionResultDto {
        DefenderExclusionResultDto {
            change,
            outcome: ScanExclusionOutcomeDto::Applied,
            folder: folder.map(String::from),
            processes: processes.iter().map(|p| p.to_string()).collect(),
            projects: None,
        }
    }

    const PROJECTS: &str = r"C:\Users\me\Desktop\Progetti";

    #[test]
    fn a_projects_folder_is_listed_with_how_to_undo_it() {
        let projects = |change| DefenderExclusionResultDto { projects: Some(PROJECTS.into()), ..defender(change, None, &[]) };
        let added = defender_exclusion(&projects(ScanExclusionChangeDto::Add));
        assert!(added.contains(&format!("  {PROJECTS} (your projects")), "{added}");
        assert!(added.contains(&format!("`lampo defender exclusion remove --projects \"{PROJECTS}\" --allow-elevation`")) && !added.contains("another PHP"), "{added}");
        let every = defender_exclusion(&DefenderExclusionResultDto { projects: Some(PROJECTS.into()), ..defender(ScanExclusionChangeDto::Add, Some(FOLDER), &[PHP]) });
        assert!(every.contains(&format!("remove --php --mysql --projects \"{PROJECTS}\" --allow-elevation")), "{every}");
        assert!(defender_exclusion(&projects(ScanExclusionChangeDto::Remove)).ends_with(&format!("  {PROJECTS} (your projects, with their vendor and node_modules folders)")));
    }

    const OUTCOMES: [(ScanExclusionOutcomeDto, &str); 4] = [
        (ScanExclusionOutcomeDto::Cancelled, "declined"),
        (ScanExclusionOutcomeDto::DefenderUnavailable, "not the active antivirus"),
        (ScanExclusionOutcomeDto::Refused, "Tamper Protection"),
        (ScanExclusionOutcomeDto::NotConfirmed, "policy"),
    ];

    #[test]
    fn a_success_names_the_folder_and_how_to_undo_it() {
        let added = scan_exclusion(&result(ScanExclusionChangeDto::Add, ScanExclusionOutcomeDto::Applied));
        assert!(added.contains(FOLDER) && added.contains("defender-exclusion remove"), "{added}");
        assert_eq!(scan_exclusion(&result(ScanExclusionChangeDto::Remove, ScanExclusionOutcomeDto::Applied)), format!("Microsoft Defender scans {FOLDER} again."));
    }

    #[test]
    fn every_other_outcome_says_why_nothing_changed() {
        for (outcome, cause) in OUTCOMES {
            let text = scan_exclusion(&result(ScanExclusionChangeDto::Add, outcome));
            assert!(text.contains(cause), "{outcome:?}: {text}");
            let text = defender_exclusion(&DefenderExclusionResultDto { outcome, ..defender(ScanExclusionChangeDto::Add, None, &[PHP]) });
            assert!(text.contains(cause) && !text.contains(PHP), "{outcome:?}: {text}");
        }
    }

    #[test]
    fn an_added_exclusion_lists_every_target_and_how_to_undo_it() {
        let text = defender_exclusion(&defender(ScanExclusionChangeDto::Add, None, &[HTTPD, PHP]));
        assert!(text.contains(&format!("the files opened by {HTTPD}")) && text.contains(&format!("the files opened by {PHP}")), "{text}");
        assert!(text.contains("`lampo defender exclusion remove --php --allow-elevation`") && text.contains("another PHP"), "{text}");
        let both = defender_exclusion(&defender(ScanExclusionChangeDto::Add, Some(FOLDER), &[PHP]));
        assert!(both.contains(FOLDER) && both.contains("remove --php --mysql --allow-elevation"), "{both}");
        let mysql = defender_exclusion(&defender(ScanExclusionChangeDto::Add, Some(FOLDER), &[]));
        assert!(mysql.contains("remove --mysql --allow-elevation") && !mysql.contains("another PHP"), "{mysql}");
    }

    #[test]
    fn commands_are_the_lines_only() {
        use lampo_application::dto::DefenderExclusionKindDto;
        let command = |kind, path: &str, line: &str| DefenderExclusionCommandDto { kind, path: path.into(), command: line.into() };
        let lines = [format!("Add-MpPreference -ExclusionPath '{FOLDER}'"), format!("Add-MpPreference -ExclusionProcess '{PHP}'")];
        let commands = [command(DefenderExclusionKindDto::Path, FOLDER, &lines[0]), command(DefenderExclusionKindDto::Process, PHP, &lines[1])];
        assert_eq!(defender_commands(&commands), lines.join("\n"));
        assert_eq!(defender_commands(&[]), "");
    }

    #[test]
    fn a_refused_addition_points_to_the_commands_for_whoever_manages_the_pc() {
        for outcome in [ScanExclusionOutcomeDto::Refused, ScanExclusionOutcomeDto::NotConfirmed] {
            let added = defender_exclusion(&DefenderExclusionResultDto { outcome, ..defender(ScanExclusionChangeDto::Add, Some(FOLDER), &[PHP]) });
            assert!(added.ends_with("Send the output of `lampo defender exclusion commands --php --mysql` to whoever manages this PC."), "{added}");
            let removed = defender_exclusion(&DefenderExclusionResultDto { outcome, ..defender(ScanExclusionChangeDto::Remove, Some(FOLDER), &[PHP]) });
            assert!(!removed.contains("exclusion commands"), "{removed}");
        }
        for outcome in [ScanExclusionOutcomeDto::Cancelled, ScanExclusionOutcomeDto::DefenderUnavailable] {
            let added = defender_exclusion(&DefenderExclusionResultDto { outcome, ..defender(ScanExclusionChangeDto::Add, None, &[PHP]) });
            assert!(!added.contains("exclusion commands"), "{outcome:?}: {added}");
        }
    }

    #[test]
    fn a_removed_exclusion_lists_every_target() {
        let text = defender_exclusion(&defender(ScanExclusionChangeDto::Remove, Some(FOLDER), &[PHP]));
        assert_eq!(text, format!("Microsoft Defender scans again:\n  {FOLDER} (the data of every MySQL instance)\n  the files opened by {PHP}"));
    }
}
