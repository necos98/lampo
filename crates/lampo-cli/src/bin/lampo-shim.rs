//! `lampo-shim.exe`, installed as `php.exe`, `composer.exe`, `mysql.exe`, `mysqldump.exe`,
//! `redis-cli.exe`: composition root of the shim (names the adapters and registers the shim
//! tools). The logic lives in `lampo_cli::shim`.
//! No async runtime, no IPC: reads the shim index file, runs the real tool, forwards its exit code.

use std::sync::Arc;

use lampo_cli::shim::{self, ShimRequest};

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let mut args = std::env::args_os();
    let argv0 = args.next().map(|a| a.to_string_lossy().into_owned()).unwrap_or_default();
    let mut tool_args = Vec::new();
    for arg in args {
        match arg.into_string() {
            Ok(arg) => tool_args.push(arg),
            Err(arg) => return shim::fail(format!("argument {arg:?} is not valid Unicode")),
        }
    }
    let paths = match lampo_infra_store::paths::resolve_lampo_paths() {
        Ok(paths) => paths,
        Err(e) => return shim::fail(e),
    };
    let request = ShimRequest {
        exe: shim::own_name(&argv0),
        args: tool_args,
        cwd: std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(),
        explicit_site: std::env::var("LAMPO_PROJECT").ok().filter(|v| !v.trim().is_empty()),
    };
    let index = lampo_infra_store::JsonShimIndex::new(paths.shim_index_file());
    let tools = lampo_domain::shim::tools::all();
    match shim::plan(&request, &tools, &index, Arc::new(lampo_infra_store::FsGitInspector)) {
        Ok(invocation) => shim::launch(&invocation),
        Err(reason) => shim::fail(reason),
    }
}
