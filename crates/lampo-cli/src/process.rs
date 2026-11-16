//! Spawning the child of `lampo exec` with the site's toolchain environment.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::{console, job};
use crate::ports::{ProgramRunner, SpawnError, SpawnSpec};

/// Finds `program` the way `cmd.exe` does: a path with a separator is used as given, a bare name
/// is searched in `dirs` in order; for each candidate the name itself (when it already has an
/// extension in `pathext`) or `name + ext` for every `PATHEXT` entry is tried. Rust's own lookup
/// only appends `.exe`, which would miss `npm.cmd`, `composer.bat` and friends.
pub fn resolve_program(program: &str, dirs: &[PathBuf], pathext: &[String], exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let candidates = |base: PathBuf| -> Option<PathBuf> {
        let has_known_ext = base.extension().and_then(OsStr::to_str).is_some_and(|e| pathext.iter().any(|p| p.trim_start_matches('.').eq_ignore_ascii_case(e)));
        if has_known_ext && exists(&base) {
            return Some(base);
        }
        pathext
            .iter()
            .map(|ext| {
                let mut name: OsString = base.clone().into_os_string();
                name.push(ext);
                PathBuf::from(name)
            })
            .find(|p| exists(p))
    };
    let has_separator = program.contains(['\\', '/']);
    if has_separator || Path::new(program).is_absolute() {
        return candidates(PathBuf::from(program));
    }
    dirs.iter().find_map(|d| candidates(d.join(program)))
}

/// `PATHEXT` entries (`.EXE`, `.CMD`...), defaulting to the Windows set.
pub fn pathext_from(value: Option<&str>) -> Vec<String> {
    let raw = value.filter(|v| !v.trim().is_empty()).unwrap_or(".COM;.EXE;.BAT;.CMD");
    raw.split(';').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// Real process runner: inherited stdio, Ctrl+C goes to the child, the child is tied to this
/// process ([`job::run_tied`]), its exit code is returned.
pub struct SystemRunner;

impl ProgramRunner for SystemRunner {
    fn run(&self, spec: &SpawnSpec) -> Result<i32, SpawnError> {
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs: Vec<PathBuf> = spec.path_prepend.iter().map(PathBuf::from).collect();
        dirs.extend(std::env::split_paths(&inherited));
        let pathext = pathext_from(std::env::var("PATHEXT").ok().as_deref());
        let program = resolve_program(&spec.program, &dirs, &pathext, &|p| p.is_file()).ok_or_else(|| SpawnError::NotFound(spec.program.clone()))?;

        let path = std::env::join_paths(dirs).map_err(|e| SpawnError::Failed(format!("cannot build PATH: {e}")))?;
        let mut command = std::process::Command::new(&program);
        command.args(&spec.args).env("PATH", path).envs(&spec.vars);

        console::survive_ctrl_c();
        let status = job::run_tied(&mut command).map_err(|e| SpawnError::Failed(format!("cannot start {}: {e}", program.display())))?;
        Ok(status.code().unwrap_or(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exists_in(files: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p| files.contains(&p.to_string_lossy().replace('/', "\\").as_str())
    }

    #[test]
    fn finds_cmd_scripts_through_pathext() {
        let dirs = vec![PathBuf::from(r"C:\tools"), PathBuf::from(r"C:\node")];
        let ext = pathext_from(None);
        let found = resolve_program("npm", &dirs, &ext, &exists_in(&[r"C:\node\npm.CMD"]));
        assert_eq!(found, Some(PathBuf::from(r"C:\node\npm.CMD")));
    }

    #[test]
    fn earlier_directories_win_and_explicit_extensions_are_kept() {
        let dirs = vec![PathBuf::from(r"C:\a"), PathBuf::from(r"C:\b")];
        let ext = pathext_from(Some(".EXE;.CMD"));
        let files = &[r"C:\a\composer.CMD", r"C:\b\composer.EXE"];
        assert_eq!(resolve_program("composer", &dirs, &ext, &exists_in(files)), Some(PathBuf::from(r"C:\a\composer.CMD")));
        assert_eq!(resolve_program("composer.cmd", &dirs, &ext, &exists_in(&[r"C:\a\composer.cmd"])), Some(PathBuf::from(r"C:\a\composer.cmd")));
    }

    #[test]
    fn paths_with_separators_skip_the_search() {
        let dirs = vec![PathBuf::from(r"C:\a")];
        let ext = pathext_from(None);
        assert_eq!(resolve_program(r"C:\x\tool", &dirs, &ext, &exists_in(&[r"C:\x\tool.EXE"])), Some(PathBuf::from(r"C:\x\tool.EXE")));
        assert_eq!(resolve_program(r"sub\tool", &dirs, &ext, &exists_in(&[r"C:\a\sub\tool.EXE"])), None);
    }

    #[test]
    fn missing_program_is_none() {
        assert_eq!(resolve_program("nope", &[PathBuf::from(r"C:\a")], &pathext_from(None), &|_| false), None);
    }

    #[test]
    fn empty_pathext_falls_back_to_the_windows_default() {
        assert_eq!(pathext_from(Some("")), [".COM", ".EXE", ".BAT", ".CMD"]);
    }
}
