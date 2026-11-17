//! Arguments of `mysql`: what concerns Lampo's MySQL data as a whole, not one database.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum MysqlCommand {
    /// Exclude Lampo's MySQL data folder from Microsoft Defender's scan, or scan it again
    ///
    /// The folder is `%LOCALAPPDATA%\Lampo\mysql` (the data of every MySQL instance) and only
    /// that: never the packages, the rest of Lampo's data, a process or an extension. Test suites
    /// rewrite its `.ibd` and redo-log files all the time and Defender scans each write; skipping
    /// them speeds the suites up. The trade-off: a malicious file saved in that folder would not
    /// be scanned there (it still is when it is copied or run from elsewhere).
    ///
    /// Each call shows a UAC prompt, so it needs `--allow-elevation` (exit 7 without it, 8 when
    /// the prompt is declined). Exit 12 when Defender is not installed or not the active
    /// antivirus, refuses the change (Tamper Protection, a group policy or Intune) or does not
    /// show it afterwards; the message says which. Defender's exclusions cannot be read without
    /// administrator rights, so Lampo cannot tell whether the exclusion is still in place.
    ///
    /// Examples:
    ///   lampo mysql defender-exclusion add --allow-elevation
    ///   lampo mysql defender-exclusion remove --allow-elevation --json
    #[command(subcommand, verbatim_doc_comment)]
    DefenderExclusion(DefenderExclusionCommand),
}

#[derive(Debug, Subcommand)]
pub enum DefenderExclusionCommand {
    /// Add the exclusion (UAC prompt; needs --allow-elevation).
    Add,
    /// Remove the exclusion (UAC prompt; needs --allow-elevation).
    Remove,
}
