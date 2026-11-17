//! The Defender exclusions of a request as PowerShell commands (`lampo defender exclusion
//! commands`, Settings), for whoever manages a PC whose exclusions a policy controls: one
//! `Add-MpPreference` (or `Remove-MpPreference`) line per target, in the order the elevated
//! helper changes them, each path a PowerShell single-quoted literal.

use std::path::Path;

use lampo_domain::scan_exclusion::{ScanExclusionChange, ScanExclusionTargets};

use crate::dto::{DefenderExclusionCommandDto, DefenderExclusionKindDto};

/// The characters PowerShell reads as a single quote: the ASCII one and the typographic ones
/// (U+2018-U+201B), which a folder name may well contain (`Progetti d’esempio`).
const SINGLE_QUOTES: [char; 5] = ['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'];

/// One command per target of `targets`: the MySQL folder, the projects folder, then the
/// executables.
pub(crate) fn exclusion_commands(targets: &ScanExclusionTargets, change: ScanExclusionChange) -> Vec<DefenderExclusionCommandDto> {
    let folders = targets.folder.iter().chain(&targets.projects).map(|folder| (DefenderExclusionKindDto::Path, folder));
    let processes = targets.processes.iter().map(|exe| (DefenderExclusionKindDto::Process, exe));
    folders.chain(processes).map(|(kind, path)| exclusion_command(change, kind, path)).collect()
}

fn exclusion_command(change: ScanExclusionChange, kind: DefenderExclusionKindDto, path: &Path) -> DefenderExclusionCommandDto {
    let cmdlet = match change {
        ScanExclusionChange::Add => "Add-MpPreference",
        ScanExclusionChange::Remove => "Remove-MpPreference",
    };
    let parameter = match kind {
        DefenderExclusionKindDto::Process => "-ExclusionProcess",
        DefenderExclusionKindDto::Path => "-ExclusionPath",
    };
    let path = path.display().to_string();
    let command = format!("{cmdlet} {parameter} {}", single_quoted(&path));
    DefenderExclusionCommandDto { kind, path, command }
}

/// `text` as a PowerShell single-quoted string: nothing in it is expanded, and each quote
/// character is doubled to stand for itself.
fn single_quoted(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for c in text.chars() {
        if SINGLE_QUOTES.contains(&c) {
            quoted.push(c);
        }
        quoted.push(c);
    }
    quoted.push('\'');
    quoted
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const MYSQL: &str = r"C:\Users\me\AppData\Local\Lampo\mysql";
    const PROJECTS: &str = r"C:\Users\me\Desktop\Progetti";
    const PHP: &str = r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe";

    fn lines(targets: &ScanExclusionTargets, change: ScanExclusionChange) -> Vec<String> {
        exclusion_commands(targets, change).into_iter().map(|c| c.command).collect()
    }

    #[test]
    fn paths_are_single_quoted_with_their_quotes_doubled() {
        assert_eq!(single_quoted(r"C:\Users\me\Desktop\Progetti"), r"'C:\Users\me\Desktop\Progetti'");
        assert_eq!(single_quoted(r"C:\Users\Mario Rossi\Lavori in corso"), r"'C:\Users\Mario Rossi\Lavori in corso'", "spaces need no escape");
        assert_eq!(single_quoted(r"C:\Users\D'Angelo\l'app"), r"'C:\Users\D''Angelo\l''app'");
        assert_eq!(single_quoted("C:\\Users\\me\\d\u{2019}esempio\u{2018}x\u{201A}\u{201B}"), "'C:\\Users\\me\\d\u{2019}\u{2019}esempio\u{2018}\u{2018}x\u{201A}\u{201A}\u{201B}\u{201B}'");
        assert_eq!(single_quoted(r"C:\$env:TEMP\`n\$(x)"), r"'C:\$env:TEMP\`n\$(x)'", "nothing is expanded in single quotes");
        assert_eq!(single_quoted("''"), "''''''");
    }

    #[test]
    fn one_command_per_target_in_the_helpers_order() {
        let targets = ScanExclusionTargets { folder: Some(PathBuf::from(MYSQL)), processes: vec![PathBuf::from(PHP)], projects: Some(PathBuf::from(PROJECTS)) };
        assert_eq!(
            exclusion_commands(&targets, ScanExclusionChange::Add),
            vec![
                DefenderExclusionCommandDto { kind: DefenderExclusionKindDto::Path, path: MYSQL.into(), command: format!("Add-MpPreference -ExclusionPath '{MYSQL}'") },
                DefenderExclusionCommandDto { kind: DefenderExclusionKindDto::Path, path: PROJECTS.into(), command: format!("Add-MpPreference -ExclusionPath '{PROJECTS}'") },
                DefenderExclusionCommandDto { kind: DefenderExclusionKindDto::Process, path: PHP.into(), command: format!("Add-MpPreference -ExclusionProcess '{PHP}'") },
            ]
        );
        assert_eq!(lines(&targets, ScanExclusionChange::Remove)[2], format!("Remove-MpPreference -ExclusionProcess '{PHP}'"));
    }

    #[test]
    fn no_target_no_command() {
        assert!(exclusion_commands(&ScanExclusionTargets::default(), ScanExclusionChange::Add).is_empty());
    }

    /// Windows PowerShell's own parser reads each command back as `Add-MpPreference` with the
    /// path, unchanged, as its only argument. The script and the paths go through the environment
    /// (UTF-16) and PowerShell compares them itself: no console encoding is involved.
    #[cfg(windows)]
    #[test]
    fn powershell_reads_the_paths_back_unchanged() {
        const CHECK: &str = r#"
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseInput($env:SCRIPT, [ref]$null, [ref]$errors)
if ($errors.Count) { exit 100 }
$read = @($ast.FindAll({ $args[0] -is [System.Management.Automation.Language.CommandAst] }, $true) |
  ForEach-Object { $_.GetCommandName() + '|' + $_.CommandElements[1].ParameterName + '|' + $_.CommandElements[2].Value + '|' + $_.CommandElements.Count })
$expected = @($env:EXPECTED -split "`n")
if ($read.Count -ne $expected.Count) { exit 101 }
for ($i = 0; $i -lt $read.Count; $i++) { if ($read[$i] -cne $expected[$i]) { exit ($i + 1) } }
exit 0"#;
        let paths = [r"C:\Users\Mario Rossi\Desktop\Progetti", r"C:\Users\D'Angelo\l''app", "C:\\Users\\me\\d\u{2019}esempio \u{2018}x\u{201B}", r"C:\Users\me\$env:TEMP\`(x);y"];
        let targets = ScanExclusionTargets { processes: paths.iter().map(PathBuf::from).collect(), ..Default::default() };
        let expected: Vec<String> = paths.iter().map(|path| format!("Add-MpPreference|ExclusionProcess|{path}|3")).collect();
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", CHECK])
            .env("SCRIPT", lines(&targets, ScanExclusionChange::Add).join("\n"))
            .env("EXPECTED", expected.join("\n"))
            .stdout(std::process::Stdio::null())
            .status()
            .expect("Windows PowerShell");
        assert_eq!(status.code(), Some(0), "1..n: that command reads back differently; 100: a parse error; 101: another number of commands");
    }
}
