//! `exec` and `env`: file-based, no app needed.

use lampo_application::{AppError, AppResult, ErrorCode};

use super::{Cmd, validation};
use crate::cli::{EnvArgs, ExecArgs, ShellKind};
use crate::ports::{SpawnError, SpawnSpec};
use crate::{render, shell};

/// Runs the child with inherited stdio and returns its exit code. `--json` does not apply: the
/// output is the child's.
pub fn exec(c: &Cmd<'_>, args: ExecArgs) -> AppResult<i32> {
    let plan = c.ctx.toolchain.exec_plan(&c.resolve_request(), &args.command)?;
    print_notice(c, plan.env.notice.as_deref());
    let spec = SpawnSpec { program: plan.program, args: plan.args, path_prepend: plan.env.path_prepend, vars: plan.env.vars };
    c.ctx.runner.run(&spec).map_err(|e| match e {
        SpawnError::NotFound(program) => {
            AppError::new(ErrorCode::NotFound, format!("command `{program}` not found in the project's toolchain or on PATH (shell built-ins need `cmd /c <command>`)"))
        }
        SpawnError::Failed(message) => AppError::new(ErrorCode::ProcessFailed, message),
    })
}

pub fn env(c: &Cmd<'_>, args: EnvArgs) -> AppResult<i32> {
    let env = c.ctx.toolchain.env(&c.resolve_request())?;
    print_notice(c, env.notice.as_deref());
    match args.shell {
        None => c.io.emit(&env, render::toolchain_env),
        Some(kind) => {
            if kind == ShellKind::Cmd {
                if let Some(bad) = shell::cmd_can_express(&env) {
                    return Err(validation(format!("cmd cannot express the value `{bad}` (quote or line break); use --shell powershell or bash")));
                }
            }
            c.io.out_line(&shell::render_env(kind, &env));
        }
    }
    Ok(0)
}

/// The resolution's note (an ignored `LAMPO_PROJECT`) as one stderr line, unless `LAMPO_QUIET`.
fn print_notice(c: &Cmd<'_>, notice: Option<&str>) {
    if let Some(notice) = notice.filter(|_| !c.ctx.env.inherited.quiet) {
        c.io.err_line(&format!("lampo: {notice}"));
    }
}
