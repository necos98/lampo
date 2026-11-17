//! The command line of the helper's Defender mode:
//! `--defender-exclusion add|remove --path <folder>`, the mode flag first. Nothing else is
//! accepted; the folder is checked by [`super::target`] on both sides.

use lampo_domain::scan_exclusion::ScanExclusionChange;

/// Selects the Defender mode; always the first argument.
pub const MODE_FLAG: &str = "--defender-exclusion";
const PATH_FLAG: &str = "--path";
const ADD: &str = "add";
const REMOVE: &str = "remove";

/// The parsed command line of the Defender mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExclusionArgs {
    pub change: ScanExclusionChange,
    /// Not trusted: [`super::target::resolve_target`] decides what it may be.
    pub path: String,
}

/// Whether `args` ask for the Defender mode (the hosts mode is the default).
pub fn is_exclusion_request(args: &[String]) -> bool {
    args.first().is_some_and(|flag| flag == MODE_FLAG)
}

impl ExclusionArgs {
    /// Exactly `--defender-exclusion <add|remove> --path <folder>`, in that order.
    pub fn parse(args: &[String]) -> Option<Self> {
        match args {
            [mode, change, path_flag, path] if mode == MODE_FLAG && path_flag == PATH_FLAG => Some(Self { change: parse_change(change)?, path: path.clone() }),
            _ => None,
        }
    }

    /// The command line [`ExclusionArgs::parse`] reads back.
    pub fn to_args(&self) -> Vec<String> {
        let change = match self.change {
            ScanExclusionChange::Add => ADD,
            ScanExclusionChange::Remove => REMOVE,
        };
        vec![MODE_FLAG.into(), change.into(), PATH_FLAG.into(), self.path.clone()]
    }
}

fn parse_change(value: &str) -> Option<ScanExclusionChange> {
    match value {
        ADD => Some(ScanExclusionChange::Add),
        REMOVE => Some(ScanExclusionChange::Remove),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLDER: &str = r"C:\Users\me\AppData\Local\Lampo\mysql";

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn argument_parsing_is_strict() {
        let add = ExclusionArgs { change: ScanExclusionChange::Add, path: FOLDER.into() };
        assert_eq!(ExclusionArgs::parse(&s(&["--defender-exclusion", "add", "--path", FOLDER])), Some(add.clone()));
        assert_eq!(ExclusionArgs::parse(&add.to_args()), Some(add), "to_args round-trips");
        let remove = ExclusionArgs { change: ScanExclusionChange::Remove, path: FOLDER.into() };
        assert_eq!(ExclusionArgs::parse(&remove.to_args()), Some(remove));

        for bad in [
            s(&[]),
            s(&["--defender-exclusion"]),
            s(&["--defender-exclusion", "add"]),
            s(&["--defender-exclusion", "add", "--path"]),
            s(&["--defender-exclusion", "ADD", "--path", FOLDER]),
            s(&["--defender-exclusion", "toggle", "--path", FOLDER]),
            s(&["--defender-exclusion", "add", "--folder", FOLDER]),
            s(&["--path", FOLDER, "--defender-exclusion", "add"]),
            s(&["--defender-exclusion", "add", "--path", FOLDER, "--path", r"C:\Windows"]),
            s(&["--defender-exclusion", "add", "--path", FOLDER, "--hosts", "h"]),
            s(&["--block", "a.json", "--defender-exclusion", "add", "--path", FOLDER]),
        ] {
            assert_eq!(ExclusionArgs::parse(&bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_a_leading_mode_flag_selects_the_mode() {
        assert!(is_exclusion_request(&s(&["--defender-exclusion", "add", "--path", FOLDER])));
        assert!(!is_exclusion_request(&s(&["--block", "a.json", "--defender-exclusion", "add"])));
        assert!(!is_exclusion_request(&s(&[])));
    }
}
