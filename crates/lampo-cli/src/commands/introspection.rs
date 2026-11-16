//! `which` and `completions`: file-based or pure, no app needed.

use lampo_application::AppResult;

use super::Cmd;
use crate::cli::{self, CompletionsArgs, WhichArgs};
use crate::render;

/// The executable name completions are registered for.
const BIN_NAME: &str = "lampo";

pub fn which(c: &Cmd<'_>, args: WhichArgs) -> AppResult<i32> {
    let which = c.ctx.shim_tools.which(&c.resolve_request(), &args.tool)?;
    c.io.emit(&which, render::which);
    Ok(0)
}

/// The completion script of the real clap definition (`--help` extras included), on stdout.
pub fn completions(c: &Cmd<'_>, args: CompletionsArgs) -> AppResult<i32> {
    let mut script = Vec::new();
    clap_complete::generate(clap_complete::Shell::from(args.shell), &mut cli::build_command(), BIN_NAME, &mut script);
    c.io.out_line(String::from_utf8_lossy(&script).trim_end_matches('\n'));
    Ok(0)
}
