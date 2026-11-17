//! Arguments of `logs`: which log, how much of it, and whether to print or open it.

use clap::{ArgGroup, Args, ValueEnum};
use lampo_domain::log_level::LevelFilter;

/// Flags that only make sense when the lines are printed, refused with `--open` / `--open-folder`.
const PRINT_ONLY_FLAGS: [&str; 4] = ["tail", "since", "level", "follow"];

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("open_target").args(["open", "open_folder"]).conflicts_with_all(PRINT_ONLY_FLAGS)))]
pub struct LogsArgs {
    /// `apache`, `apache-error`, `mysql-8.4`, `mysql-8.4-error`, `redis`, `access`, `error`,
    /// `php` (the last three for the resolved site, or `access:<site>` ...). Default: list
    /// the available sources.
    pub source: Option<String>,
    /// Number of last lines to print (the level filter applies to these lines).
    #[arg(long, short = 'n', default_value_t = 100)]
    pub tail: usize,
    /// Only lines since: `30s`, `10m`, `2h`, `1d` or a Unix timestamp in milliseconds.
    #[arg(long)]
    pub since: Option<String>,
    /// Only lines of this severity, as the app's log viewer: every line, warnings and errors, or
    /// errors only (error/fatal/critical/emerg/alert words and HTTP 5xx statuses are errors;
    /// warn/warning/deprecated/notice words are warnings).
    #[arg(long, value_enum, default_value_t = LogLevelArg::All)]
    pub level: LogLevelArg,
    /// Keep printing new lines until Ctrl+C (JSON: one `LogLineDto` per line, NDJSON).
    #[arg(long, short = 'f')]
    pub follow: bool,
    /// Open the log file in the app associated with it instead of printing it.
    #[arg(long, requires = "source")]
    pub open: bool,
    /// Open the folder holding the log file in Explorer instead of printing it.
    #[arg(long, requires = "source")]
    pub open_folder: bool,
}

/// `--level`: the filters of the app's log viewer («Tutti», «Avvisi ed errori», «Solo errori»).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogLevelArg {
    /// Every line.
    All,
    /// Warnings and errors.
    Warnings,
    /// Errors only.
    Errors,
}

impl From<LogLevelArg> for LevelFilter {
    fn from(level: LogLevelArg) -> Self {
        match level {
            LogLevelArg::All => Self::All,
            LogLevelArg::Warnings => Self::Warnings,
            LogLevelArg::Errors => Self::Errors,
        }
    }
}
