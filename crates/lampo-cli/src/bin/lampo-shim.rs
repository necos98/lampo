//! `lampo-shim.exe`, installed as `php.exe`, `composer.exe`, `mysql.exe`, `mysqldump.exe`,
//! `redis-cli.exe`: composition root of the shim (names the adapters and registers the shim
//! tools). The logic lives in `lampo_cli::shim`.
//! No async runtime, no IPC: reads the shim index file, runs the real tool, forwards its exit code.

use std::sync::Arc;

use lampo_cli::inherited::InheritedEnv;
use lampo_cli::shim::{self, ShimFacts, ShimRequest};
use lampo_domain::paths::LampoPaths;

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
    // Only the data root: the shim never needs the config root.
    let data_root = match lampo_infra_store::paths::resolve_data_root() {
        Ok(root) => root,
        Err(e) => return shim::fail(e),
    };
    let inherited = InheritedEnv::from_process();
    let request = ShimRequest {
        exe: shim::own_name(&argv0),
        args: tool_args,
        cwd: std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(),
        inherited_site: inherited.site,
        inherited_root: inherited.root,
    };
    let index = lampo_infra_store::JsonShimIndex::new(LampoPaths::shim_index_file_in(&data_root));
    let tools = lampo_domain::shim::tools::all();
    let facts = ShimFacts {
        git: Arc::new(lampo_infra_store::FsGitInspector),
        canonicalizer: Arc::new(lampo_infra_store::FsPathCanonicalizer),
        folders: Arc::new(lampo_infra_store::LocalFileSystem),
    };
    match shim::plan(&request, &tools, &index, facts) {
        Ok(planned) => {
            if !inherited.quiet {
                planned.notices.iter().for_each(shim::note);
            }
            shim::launch(&planned.invocation)
        }
        Err(reason) => shim::fail(reason),
    }
}
