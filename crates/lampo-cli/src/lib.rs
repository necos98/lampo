//! # lampo-cli
//!
//! The `lampo` command-line front-end and the `lampo-shim` executable.
//!
//! * `cli` - the clap surface; `commands` - one thin handler per command;
//! * `ports` / `context` - what the handlers need, as role traits (faked in tests);
//! * `local` / `composition` - production implementations and the only place naming adapters;
//! * `render` / `io` / `error_doc` / `exit_code` - output, JSON error shape, exit codes;
//! * `shim` - the stateless tool launcher behind `php.exe`, `composer.exe`, ...

pub mod cli;
pub mod commands;
pub mod composition;
pub mod console;
pub mod context;
pub mod error_doc;
pub mod exit_code;
pub mod io;
pub mod local;
pub mod ports;
pub mod process;
pub mod progress;
pub mod render;
pub mod shell;
pub mod shim;

#[cfg(test)]
mod testing;

use std::ffi::OsString;
use std::sync::Mutex;

use clap::FromArgMatches;
use clap::error::ErrorKind;
use lampo_application::AppError;

use crate::cli::Cli;
use crate::io::Io;

/// Whether the command line asks for JSON (so that even usage errors are JSON). Arguments after
/// a bare `--` belong to `exec` and are not ours.
pub fn wants_json(args: &[OsString]) -> bool {
    args.iter().skip(1).take_while(|a| *a != "--").any(|a| a == "--json")
}

static PANIC_MESSAGE: Mutex<String> = Mutex::new(String::new());

/// Entry point of the `lampo` binary: parses, runs, reports; returns the process exit code.
/// Never panics out: an unexpected panic (a bug, or a not-yet-implemented adapter) becomes an
/// `internal` error in the normal error shape.
pub fn run(args: Vec<OsString>) -> i32 {
    let json = wants_json(&args);
    let io = Io::stdio(json);

    let parsed = cli::build_command().try_get_matches_from(&args).and_then(|m| Cli::from_arg_matches(&m));
    let cli = match parsed {
        Ok(cli) => cli,
        Err(e) if json && !matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => return error_doc::report_usage(&io, &e.render().to_string()),
        Err(e) => {
            let _ = e.print();
            return e.exit_code();
        }
    };

    // The Claude Code hook must never disturb a session: any failure is silent, exit 0.
    let hook = matches!(&cli.command, cli::Command::Agent(cli::AgentCommand::Context(a)) if a.hook);
    std::panic::set_hook(Box::new(|info| {
        if let Ok(mut slot) = PANIC_MESSAGE.lock() {
            *slot = info.to_string();
        }
    }));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(cli, &io)));
    match outcome {
        Ok(Ok(code)) => code,
        _ if hook => 0,
        Ok(Err(error)) => error_doc::report(&io, &error),
        Err(_) => {
            let message = PANIC_MESSAGE.lock().map(|m| m.clone()).unwrap_or_default();
            error_doc::report(&io, &AppError::internal(format!("unexpected failure: {message}")))
        }
    }
}

fn execute(cli: Cli, io: &Io) -> Result<i32, AppError> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| AppError::internal(format!("cannot start the async runtime: {e}")))?;
    runtime.block_on(async {
        let ctx = composition::build()?;
        commands::run(cli, &ctx, io).await
    })
}
