//! The command line of the helper's Defender mode, the mode flag first:
//!
//! ```text
//! --defender-exclusion add|remove [--path <folder>] [--process <exe>]...
//! ```
//!
//! At least one target; the folder (at most one) comes before the executables. The MySQL-only
//! form `--defender-exclusion add|remove --path <folder>` is unchanged. Nothing else is accepted;
//! the targets are checked by [`super::target`] and [`super::executable`] on both sides.

use lampo_domain::scan_exclusion::ScanExclusionChange;

/// Selects the Defender mode; always the first argument.
pub const MODE_FLAG: &str = "--defender-exclusion";
const PATH_FLAG: &str = "--path";
const PROCESS_FLAG: &str = "--process";
const ADD: &str = "add";
const REMOVE: &str = "remove";
/// Most executables one request may name: far more than the PHP series and Apache Lampo
/// installs, and far below the length limit of a command line.
pub const MAX_PROCESSES: usize = 64;

/// The parsed command line of the Defender mode. Nothing in it is trusted:
/// [`super::target::resolve_target`] and [`super::executable::resolve_executable`] decide what
/// the paths may be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExclusionArgs {
    pub change: ScanExclusionChange,
    pub path: Option<String>,
    pub processes: Vec<String>,
}

/// Whether `args` ask for the Defender mode (the hosts mode is the default).
pub fn is_exclusion_request(args: &[String]) -> bool {
    args.first().is_some_and(|flag| flag == MODE_FLAG)
}

impl ExclusionArgs {
    /// Exactly the form of the module docs.
    pub fn parse(args: &[String]) -> Option<Self> {
        let [mode, change, rest @ ..] = args else { return None };
        if mode != MODE_FLAG {
            return None;
        }
        let change = parse_change(change)?;
        let (path, rest) = match rest {
            [flag, path, rest @ ..] if flag == PATH_FLAG => (Some(path.clone()), rest),
            _ => (None, rest),
        };
        let processes = parse_processes(rest)?;
        (path.is_some() || !processes.is_empty()).then_some(Self { change, path, processes })
    }

    /// The command line [`ExclusionArgs::parse`] reads back.
    pub fn to_args(&self) -> Vec<String> {
        let change = match self.change {
            ScanExclusionChange::Add => ADD,
            ScanExclusionChange::Remove => REMOVE,
        };
        let mut args = vec![MODE_FLAG.to_string(), change.to_string()];
        if let Some(path) = &self.path {
            args.extend([PATH_FLAG.to_string(), path.clone()]);
        }
        for process in &self.processes {
            args.extend([PROCESS_FLAG.to_string(), process.clone()]);
        }
        args
    }
}

fn parse_change(value: &str) -> Option<ScanExclusionChange> {
    match value {
        ADD => Some(ScanExclusionChange::Add),
        REMOVE => Some(ScanExclusionChange::Remove),
        _ => None,
    }
}

/// `--process <exe>` pairs only, at most [`MAX_PROCESSES`].
fn parse_processes(args: &[String]) -> Option<Vec<String>> {
    if args.len() % 2 != 0 || args.len() / 2 > MAX_PROCESSES {
        return None;
    }
    args.chunks_exact(2).map(|pair| (pair[0] == PROCESS_FLAG).then(|| pair[1].clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLDER: &str = r"C:\Users\me\AppData\Local\Lampo\mysql";
    const PHP: &str = r"C:\Users\me\AppData\Local\Lampo\packages\php-8.3\php-cgi.exe";
    const HTTPD: &str = r"C:\Users\me\AppData\Local\Lampo\packages\apache-2.4\bin\httpd.exe";

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn request(change: ScanExclusionChange, path: Option<&str>, processes: &[&str]) -> ExclusionArgs {
        ExclusionArgs { change, path: path.map(str::to_string), processes: processes.iter().map(|p| p.to_string()).collect() }
    }

    #[test]
    fn the_mysql_form_is_unchanged() {
        let add = request(ScanExclusionChange::Add, Some(FOLDER), &[]);
        assert_eq!(ExclusionArgs::parse(&s(&["--defender-exclusion", "add", "--path", FOLDER])), Some(add.clone()));
        assert_eq!(add.to_args(), s(&["--defender-exclusion", "add", "--path", FOLDER]));
        let remove = request(ScanExclusionChange::Remove, Some(FOLDER), &[]);
        assert_eq!(ExclusionArgs::parse(&remove.to_args()), Some(remove));
    }

    #[test]
    fn executables_follow_the_folder() {
        let processes = request(ScanExclusionChange::Add, None, &[PHP, HTTPD]);
        assert_eq!(ExclusionArgs::parse(&s(&["--defender-exclusion", "add", "--process", PHP, "--process", HTTPD])), Some(processes.clone()));
        assert_eq!(ExclusionArgs::parse(&processes.to_args()), Some(processes), "to_args round-trips");
        let both = request(ScanExclusionChange::Remove, Some(FOLDER), &[PHP]);
        assert_eq!(both.to_args(), s(&["--defender-exclusion", "remove", "--path", FOLDER, "--process", PHP]));
        assert_eq!(ExclusionArgs::parse(&both.to_args()), Some(both));
        let most: Vec<String> = (0..MAX_PROCESSES).map(|i| format!(r"C:\x\{i}.exe")).collect();
        let most = ExclusionArgs { change: ScanExclusionChange::Add, path: None, processes: most };
        assert_eq!(ExclusionArgs::parse(&most.to_args()), Some(most));
    }

    #[test]
    fn argument_parsing_is_strict() {
        for bad in [
            s(&[]),
            s(&["--defender-exclusion"]),
            s(&["--defender-exclusion", "add"]),
            s(&["--defender-exclusion", "add", "--path"]),
            s(&["--defender-exclusion", "add", "--process"]),
            s(&["--defender-exclusion", "ADD", "--path", FOLDER]),
            s(&["--defender-exclusion", "toggle", "--path", FOLDER]),
            s(&["--defender-exclusion", "add", "--folder", FOLDER]),
            s(&["--defender-exclusion", "add", "--extension", ".php"]),
            s(&["--path", FOLDER, "--defender-exclusion", "add"]),
            s(&["--defender-exclusion", "add", "--path", FOLDER, "--path", r"C:\Windows"]),
            s(&["--defender-exclusion", "add", "--process", PHP, "--path", FOLDER]),
            s(&["--defender-exclusion", "add", "--process", PHP, "--process"]),
            s(&["--defender-exclusion", "add", "--process", PHP, PHP]),
            s(&["--defender-exclusion", "add", "--path", FOLDER, "--hosts", "h"]),
            s(&["--block", "a.json", "--defender-exclusion", "add", "--path", FOLDER]),
        ] {
            assert_eq!(ExclusionArgs::parse(&bad), None, "{bad:?}");
        }
        let too_many: Vec<String> = (0..=MAX_PROCESSES).map(|i| format!(r"C:\x\{i}.exe")).collect();
        let too_many = ExclusionArgs { change: ScanExclusionChange::Add, path: None, processes: too_many };
        assert_eq!(ExclusionArgs::parse(&too_many.to_args()), None, "more than {MAX_PROCESSES} executables");
    }

    #[test]
    fn only_a_leading_mode_flag_selects_the_mode() {
        assert!(is_exclusion_request(&s(&["--defender-exclusion", "add", "--path", FOLDER])));
        assert!(!is_exclusion_request(&s(&["--block", "a.json", "--defender-exclusion", "add"])));
        assert!(!is_exclusion_request(&s(&[])));
    }
}
