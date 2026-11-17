//! Arguments of `defender`: Lampo's optional Microsoft Defender exclusions.

use clap::{Args, Subcommand};

#[derive(Debug, Subcommand)]
pub enum DefenderCommand {
    /// Exclude Lampo's servers, MySQL data and/or a projects folder from Microsoft Defender's
    /// scan, or scan them again
    ///
    /// --php: one process exclusion for `php-cgi.exe` of every PHP installed by Lampo and one
    /// for Apache's `bin\httpd.exe` (`%LOCALAPPDATA%\Lampo\packages\...`, never a wildcard).
    /// Defender then skips the files those processes open: the first request to a site no
    /// longer waits seconds for Defender to scan its PHP sources (which it does again after
    /// every definition update, reboot or change), for every project wherever it is, and no
    /// source folder is excluded. The executables are those installed now: after installing
    /// another PHP, run `add --php` again. The trade-off: a malicious file read by PHP or
    /// Apache is not scanned at that moment (it still is when written by another program,
    /// copied or run).
    ///
    /// --mysql: the folder `%LOCALAPPDATA%\Lampo\mysql` (the data of every MySQL instance),
    /// which test suites rewrite all the time; same as `lampo mysql defender-exclusion`.
    ///
    /// --projects [<folder>]: a folder holding your projects (e.g. `%USERPROFILE%\Desktop\
    /// Progetti`), which also speeds up `composer install`, `git checkout`, `npm install` and
    /// the IDE. The heaviest trade-off: downloaded code (`vendor\`, `node_modules\`) lands
    /// there and is never scanned, so prefer --php unless those tools are slow. Without a
    /// folder, the deepest folder holding every registered project that may be excluded (the
    /// one Settings suggests); exit 4 when there is none. The folder must exist
    /// and be inside your user profile, never the profile itself, AppData, a Start Menu, the
    /// roots of Desktop, Documents, Downloads or OneDrive, a drive root, Windows, Program
    /// Files or ProgramData, nor go through a junction or link (exit 4 otherwise).
    ///
    /// Each call shows one UAC prompt whatever it names, so it needs `--allow-elevation` (exit
    /// 7 without it, 8 when the prompt is declined). Exit 10 for --php with no PHP or Apache
    /// installed. Exit 12 when Defender is not installed or not the active antivirus, refuses
    /// the change (Tamper Protection, a group policy or Intune) or does not show it afterwards;
    /// the message says which. Defender's exclusions cannot be read without administrator
    /// rights, so Lampo cannot tell whether they are still in place. Lampo never adds them on
    /// its own.
    ///
    /// Examples:
    ///   lampo defender exclusion add --php --allow-elevation
    ///   lampo defender exclusion add --php --mysql --allow-elevation
    ///   lampo defender exclusion add --projects --allow-elevation
    ///   lampo defender exclusion add --projects "C:\Users\me\Desktop\Progetti" --allow-elevation
    ///   lampo defender exclusion remove --php --allow-elevation --json
    #[command(subcommand, verbatim_doc_comment)]
    Exclusion(ExclusionCommand),
}

#[derive(Debug, Subcommand)]
pub enum ExclusionCommand {
    /// Add the exclusions (one UAC prompt; needs --allow-elevation).
    Add(ExclusionTargets),
    /// Remove the exclusions (one UAC prompt; needs --allow-elevation).
    Remove(ExclusionTargets),
    /// Print the PowerShell commands that add the exclusions, for whoever manages this PC
    ///
    /// When Defender refuses the change or does not show it (exit 12: a group policy or Intune
    /// controls its exclusions), send these lines to your IT department: one `Add-MpPreference
    /// -ExclusionProcess '<exe>'` or `-ExclusionPath '<folder>'` per exclusion, with the exact
    /// paths `add` would name on this PC, quoted for PowerShell. Without a selector, --php. No
    /// UAC prompt, nothing is changed; the same checks and exit codes as `add`.
    ///
    /// Examples:
    ///   lampo defender exclusion commands
    ///   lampo defender exclusion commands --php --mysql --projects "C:\Users\me\Desktop\Progetti"
    ///   lampo defender exclusion commands --json
    #[command(verbatim_doc_comment)]
    Commands(CommandTargets),
}

/// What the request names: at least one.
#[derive(Debug, Args)]
#[group(required = true, multiple = true)]
pub struct ExclusionTargets {
    /// php-cgi.exe of every installed PHP and Apache's httpd.exe (process exclusions).
    #[arg(long)]
    pub php: bool,
    /// The MySQL data folder (%LOCALAPPDATA%\Lampo\mysql).
    #[arg(long)]
    pub mysql: bool,
    /// A folder holding your projects; without a value, the one suggested from the registered projects.
    #[arg(long, value_name = "FOLDER", num_args = 0..=1)]
    pub projects: Option<Option<String>>,
}

/// What the commands exclude: the selectors of `add`, --php when none is named.
#[derive(Debug, Args)]
pub struct CommandTargets {
    /// php-cgi.exe of every installed PHP and Apache's httpd.exe (process exclusions; the default).
    #[arg(long)]
    pub php: bool,
    /// The MySQL data folder (%LOCALAPPDATA%\Lampo\mysql).
    #[arg(long)]
    pub mysql: bool,
    /// A folder holding your projects; without a value, the one suggested from the registered projects.
    #[arg(long, value_name = "FOLDER", num_args = 0..=1)]
    pub projects: Option<Option<String>>,
}

impl CommandTargets {
    /// The targets named, --php when none is.
    pub fn or_php(self) -> ExclusionTargets {
        let php = self.php || (!self.mysql && self.projects.is_none());
        ExclusionTargets { php, mysql: self.mysql, projects: self.projects }
    }
}
