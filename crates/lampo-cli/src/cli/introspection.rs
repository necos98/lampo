//! Arguments of the commands that explain the CLI itself: `which` and `completions`.

use clap::builder::PossibleValuesParser;
use clap::{Args, ValueEnum};

#[derive(Debug, Args)]
pub struct WhichArgs {
    /// The shimmed tool.
    #[arg(value_parser = shim_tool_names(), ignore_case = true, value_name = "TOOL")]
    pub tool: String,
}

/// The names of the registered shim tools (`php`, `composer`, `mysql`, `mysqldump`, `redis-cli`).
fn shim_tool_names() -> PossibleValuesParser {
    PossibleValuesParser::new(lampo_domain::shim::tools::all().iter().map(|t| t.name()))
}

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// The shell to complete in.
    #[arg(value_enum, conflicts_with = "json")]
    pub shell: CompletionShell,
}

/// `lampo completions <shell>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CompletionShell {
    Powershell,
    Bash,
    Zsh,
    Fish,
}

impl From<CompletionShell> for clap_complete::Shell {
    fn from(shell: CompletionShell) -> Self {
        match shell {
            CompletionShell::Powershell => Self::PowerShell,
            CompletionShell::Bash => Self::Bash,
            CompletionShell::Zsh => Self::Zsh,
            CompletionShell::Fish => Self::Fish,
        }
    }
}
