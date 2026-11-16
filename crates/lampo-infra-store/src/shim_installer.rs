use std::path::{Path, PathBuf};

use lampo_domain::ports::ShimInstaller;
use lampo_domain::shim::tools;
use lampo_domain::{PortError, PortResult};
use lampo_infra_fs::ensure_dir;

/// Copies `lampo-shim.exe` once per tool name (`lampo_domain::shim::tools::all()`), e.g.
/// `php.exe`, `composer.exe`, `mysql.exe`, `mysqldump.exe`, `redis-cli.exe`, and optionally
/// the `lampo.exe` CLI under its own name ([`ExeShimInstaller::with_cli`]), so that the one
/// PATH entry of the shim dir provides the CLI too. Hard links are not used: the app folder and
/// `%LOCALAPPDATA%` may be on different volumes.
pub struct ExeShimInstaller {
    /// The `lampo-shim.exe` shipped next to the app executable.
    shim_executable: PathBuf,
    tool_names: Vec<&'static str>,
    /// The `lampo.exe` shipped next to the app executable. Skipped while missing (a development
    /// build that did not build the CLI): the shims do not need it.
    cli_executable: Option<PathBuf>,
}

impl ExeShimInstaller {
    /// `tool_names` are tool names (`php`) or file names (`php.exe`); `.exe` is added if missing.
    pub fn new(shim_executable: PathBuf, tool_names: Vec<&'static str>) -> Self {
        Self { shim_executable, tool_names, cli_executable: None }
    }

    /// Also copies `cli_executable` (`lampo.exe`) into the shim dir, under its own file name.
    pub fn with_cli(mut self, cli_executable: PathBuf) -> Self {
        self.cli_executable = Some(cli_executable);
        self
    }

    /// Installer for every built-in shim tool.
    pub fn for_builtin_tools(shim_executable: PathBuf) -> Self {
        Self::new(shim_executable, tools::all().iter().map(|t| t.name()).collect())
    }

    fn shim_targets(&self, shim_dir: &Path) -> Vec<PathBuf> {
        self.tool_names.iter().map(|n| if n.to_ascii_lowercase().ends_with(".exe") { (*n).to_string() } else { format!("{n}.exe") }).map(|file| shim_dir.join(file)).collect()
    }

    /// `(source, target)` of the CLI copy, when configured.
    fn cli_copy(&self, shim_dir: &Path) -> Option<(&Path, PathBuf)> {
        let cli = self.cli_executable.as_deref()?;
        Some((cli, shim_dir.join(cli.file_name()?)))
    }

    /// Every copy to keep in sync: one per tool, plus the CLI while its executable exists.
    fn copies(&self, shim_dir: &Path) -> Vec<(&Path, PathBuf)> {
        let mut copies: Vec<_> = self.shim_targets(shim_dir).into_iter().map(|t| (self.shim_executable.as_path(), t)).collect();
        match self.cli_copy(shim_dir) {
            Some((cli, target)) if cli.is_file() => copies.push((cli, target)),
            Some((cli, _)) => tracing::warn!("{} not found: the CLI is not copied into the shim dir", cli.display()),
            None => {}
        }
        copies
    }

    /// Every file a copy may occupy, the CLI one included even when its executable is missing.
    fn copy_targets(&self, shim_dir: &Path) -> Vec<PathBuf> {
        let mut targets = self.shim_targets(shim_dir);
        targets.extend(self.cli_copy(shim_dir).map(|(_, target)| target));
        targets
    }
}

/// Start of the extension of a copy renamed away by [`place_copy`]; the process id follows.
const MOVED_ASIDE_EXTENSION_PREFIX: &str = "old-";

fn moved_aside_path(target: &Path) -> PathBuf {
    target.with_extension(format!("{MOVED_ASIDE_EXTENSION_PREFIX}{}", std::process::id()))
}

/// Whether `file_name` is a copy that [`place_copy`] renamed away (`php.old-1234`) from one of
/// `targets`.
fn is_moved_aside(file_name: &str, targets: &[PathBuf]) -> bool {
    let Some((stem, extension)) = file_name.rsplit_once('.') else {
        return false;
    };
    let is_aside_extension = extension.strip_prefix(MOVED_ASIDE_EXTENSION_PREFIX).is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()));
    is_aside_extension && targets.iter().filter_map(|t| t.file_stem()).any(|target_stem| target_stem.eq_ignore_ascii_case(stem))
}

/// Places `source` at `target` without ever leaving a partial `target`. A running shim cannot be
/// overwritten on Windows, but it can be renamed away, so on failure the old file is moved aside.
fn place_copy(source: &Path, target: &Path) -> PortResult<()> {
    let context = || format!("installing {}", target.display());
    let dir = target.parent().unwrap_or(Path::new("."));
    let tmp = tempfile::Builder::new().prefix(".lampo-").suffix(".tmp").tempfile_in(dir).map_err(|e| PortError::io(context(), e))?;
    std::fs::copy(source, tmp.path()).map_err(|e| PortError::io(context(), e))?;
    let tmp = match tmp.persist(target) {
        Ok(_) => return Ok(()),
        Err(e) => e.file,
    };
    let aside = moved_aside_path(target);
    std::fs::rename(target, &aside).map_err(|e| PortError::io(context(), format!("{e} (is the shim running?)")))?;
    match tmp.persist(target) {
        Ok(_) => {
            let _ = std::fs::remove_file(&aside); // fails while the old shim still runs; harmless
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::rename(&aside, target);
            Err(PortError::io(context(), e.error))
        }
    }
}

/// Why deleting a copy in `shim_dir` most likely failed: `lampo` typed in a terminal is the copy
/// in the shim dir, which cannot delete itself; otherwise another shim or CLI copy is running.
fn locked_copy_hint(shim_dir: &Path) -> &'static str {
    let canonical_dir = |dir: &Path| std::fs::canonicalize(dir).ok();
    let running_inside = std::env::current_exe().ok().and_then(|exe| exe.parent().and_then(canonical_dir)).is_some_and(|exe_dir| Some(exe_dir) == canonical_dir(shim_dir));
    if running_inside { "this lampo is the copy in that folder: run the lampo.exe in the Lampo install folder instead" } else { "is a shim or the lampo copy in it running?" }
}

/// Deletes one copy; a missing one is already uninstalled. A running copy cannot be deleted on
/// Windows, so that failure names the likely cause.
fn remove_copy(target: &Path) -> PortResult<()> {
    match std::fs::remove_file(target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            let hint = locked_copy_hint(target.parent().unwrap_or(Path::new(".")));
            Err(PortError::io(format!("deleting {}", target.display()), format!("{e} ({hint})")))
        }
    }
}

/// Best effort, like in [`place_copy`]: a copy moved aside while it ran may be running still, and
/// it is harmless once its folder is off the PATH.
fn remove_copies_moved_aside(shim_dir: &Path, targets: &[PathBuf]) {
    let entries = match std::fs::read_dir(shim_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            tracing::warn!("listing {}: {e}", shim_dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        if entry.file_name().to_str().is_some_and(|name| is_moved_aside(name, targets))
            && let Err(e) = std::fs::remove_file(entry.path())
        {
            tracing::debug!("{} left in place: {e}", entry.path().display());
        }
    }
}

/// Removes `dir` when nothing else is left in it (the shim index usually is); missing is fine.
fn remove_dir_if_empty(dir: &Path) -> PortResult<()> {
    match std::fs::remove_dir(dir) {
        Ok(()) => Ok(()),
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty) => Ok(()),
        Err(e) => Err(PortError::io(format!("deleting {}", dir.display()), e)),
    }
}

fn same_content(a: &Path, b: &Path) -> bool {
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

impl ExeShimInstaller {
    /// Copies the shim over every tool that differs from it (and the CLI over its copy); returns
    /// whether anything changed.
    fn sync_copies(&self, shim_dir: &Path) -> PortResult<bool> {
        if !self.shim_executable.is_file() {
            return Err(PortError::not_found("shim executable", self.shim_executable.display().to_string()));
        }
        ensure_dir(shim_dir)?;
        let mut changed = false;
        for (source, target) in self.copies(shim_dir) {
            if !same_content(source, &target) {
                place_copy(source, &target)?;
                changed = true;
            }
        }
        Ok(changed)
    }
}

impl ShimInstaller for ExeShimInstaller {
    /// Idempotent: tools already identical to the shim are left untouched.
    fn install(&self, shim_dir: &Path) -> PortResult<()> {
        self.sync_copies(shim_dir).map(|_| ())
    }

    /// Stale copies (and tools or the CLI copy added by the update) are replaced only when at
    /// least one copy is already installed; byte comparison, so a new shim with the same size is
    /// caught too.
    fn refresh(&self, shim_dir: &Path) -> PortResult<bool> {
        let cli_target = self.cli_copy(shim_dir).map(|(_, target)| target);
        if !self.shim_targets(shim_dir).iter().chain(cli_target.iter()).any(|t| t.is_file()) {
            return Ok(false);
        }
        self.sync_copies(shim_dir)
    }

    /// Deletes the copies only: the shim index next to them is the app's (the CLI resolves the
    /// site with it) and stays, like any file Lampo did not put there. The folder goes once empty.
    fn uninstall(&self, shim_dir: &Path) -> PortResult<()> {
        let targets = self.copy_targets(shim_dir);
        let mut first_error = None;
        for target in &targets {
            if let Err(e) = remove_copy(target) {
                first_error.get_or_insert(e);
            }
        }
        remove_copies_moved_aside(shim_dir, &targets);
        if let Some(e) = first_error {
            return Err(e);
        }
        remove_dir_if_empty(shim_dir)
    }

    /// True when every copy exists and has the size of its executable (so an app update that
    /// ships a new shim or CLI reports `false` until refreshed or reinstalled).
    fn is_installed(&self, shim_dir: &Path) -> PortResult<bool> {
        Ok(self.copies(shim_dir).iter().all(|(source, target)| {
            let expected = std::fs::metadata(source).ok().map(|m| m.len());
            match std::fs::metadata(target) {
                Ok(m) => m.is_file() && expected.is_none_or(|len| m.len() == len),
                Err(_) => false,
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("lampo-shim.exe");
        std::fs::write(&exe, b"shim-v1").unwrap();
        let bin = dir.path().join("bin");
        (dir, exe, bin)
    }

    #[test]
    fn installs_one_copy_per_tool_and_is_idempotent() {
        let (_d, exe, bin) = setup();
        let installer = ExeShimInstaller::for_builtin_tools(exe);
        assert!(!installer.is_installed(&bin).unwrap());
        installer.install(&bin).unwrap();
        installer.install(&bin).unwrap();
        assert!(installer.is_installed(&bin).unwrap());
        for name in ["php.exe", "composer.exe", "mysql.exe", "mysqldump.exe", "redis-cli.exe"] {
            assert_eq!(std::fs::read(bin.join(name)).unwrap(), b"shim-v1", "{name}");
        }
        assert_eq!(std::fs::read_dir(&bin).unwrap().count(), 5);
    }

    #[test]
    fn upgrade_replaces_copies_and_detects_stale_ones() {
        let (_d, exe, bin) = setup();
        let installer = ExeShimInstaller::new(exe.clone(), vec!["php", "composer.exe"]);
        installer.install(&bin).unwrap();
        std::fs::write(&exe, b"shim-version-2").unwrap();
        assert!(!installer.is_installed(&bin).unwrap());
        installer.install(&bin).unwrap();
        assert!(installer.is_installed(&bin).unwrap());
        assert_eq!(std::fs::read(bin.join("composer.exe")).unwrap(), b"shim-version-2");
    }

    #[test]
    fn refresh_replaces_stale_copies_after_an_update() {
        let (_d, exe, bin) = setup();
        let installer = ExeShimInstaller::new(exe.clone(), vec!["php", "composer"]);
        installer.install(&bin).unwrap();
        assert!(!installer.refresh(&bin).unwrap(), "up to date: nothing to do");
        // Same size, different bytes: a size check alone would miss it.
        std::fs::write(&exe, b"shim-v2").unwrap();
        std::fs::remove_file(bin.join("composer.exe")).unwrap();
        assert!(installer.refresh(&bin).unwrap());
        assert_eq!(std::fs::read(bin.join("php.exe")).unwrap(), b"shim-v2");
        assert_eq!(std::fs::read(bin.join("composer.exe")).unwrap(), b"shim-v2", "missing tools are added back");
        assert!(!installer.refresh(&bin).unwrap());
    }

    #[test]
    fn refresh_does_nothing_when_the_shims_were_never_installed() {
        let (_d, exe, bin) = setup();
        let installer = ExeShimInstaller::new(exe, vec!["php"]);
        assert!(!installer.refresh(&bin).unwrap());
        assert!(!bin.exists());
    }

    fn with_cli(exe: &Path, dir: &Path) -> (ExeShimInstaller, PathBuf) {
        let cli = dir.join("app").join("lampo.exe");
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        std::fs::write(&cli, b"cli-v1").unwrap();
        (ExeShimInstaller::new(exe.to_path_buf(), vec!["php", "composer"]).with_cli(cli.clone()), cli)
    }

    #[test]
    fn install_copies_the_cli_next_to_the_shims() {
        let (d, exe, bin) = setup();
        let (installer, _cli) = with_cli(&exe, d.path());
        installer.install(&bin).unwrap();
        assert!(installer.is_installed(&bin).unwrap());
        assert_eq!(std::fs::read(bin.join("lampo.exe")).unwrap(), b"cli-v1");
        assert_eq!(std::fs::read(bin.join("php.exe")).unwrap(), b"shim-v1");
        std::fs::remove_file(bin.join("lampo.exe")).unwrap();
        assert!(!installer.is_installed(&bin).unwrap(), "a missing CLI copy counts as not installed");
    }

    #[test]
    fn refresh_updates_the_cli_copy_and_adds_it_to_older_installs() {
        let (d, exe, bin) = setup();
        // Installed by a version that did not copy the CLI yet.
        ExeShimInstaller::new(exe.clone(), vec!["php", "composer"]).install(&bin).unwrap();
        let (installer, cli) = with_cli(&exe, d.path());
        assert!(!installer.is_installed(&bin).unwrap());
        assert!(installer.refresh(&bin).unwrap());
        assert_eq!(std::fs::read(bin.join("lampo.exe")).unwrap(), b"cli-v1");
        assert!(!installer.refresh(&bin).unwrap(), "up to date");
        std::fs::write(&cli, b"cli-v2").unwrap(); // same size, new bytes: an app update
        assert!(installer.refresh(&bin).unwrap());
        assert_eq!(std::fs::read(bin.join("lampo.exe")).unwrap(), b"cli-v2");
        assert_eq!(std::fs::read(bin.join("php.exe")).unwrap(), b"shim-v1", "shims untouched");
    }

    #[test]
    fn refresh_does_not_install_the_cli_alone() {
        let (d, exe, bin) = setup();
        let (installer, _cli) = with_cli(&exe, d.path());
        assert!(!installer.refresh(&bin).unwrap());
        assert!(!bin.exists());
    }

    #[test]
    fn a_missing_cli_executable_is_skipped() {
        let (d, exe, bin) = setup();
        let installer = ExeShimInstaller::new(exe, vec!["php"]).with_cli(d.path().join("missing").join("lampo.exe"));
        installer.install(&bin).unwrap();
        assert!(installer.is_installed(&bin).unwrap());
        assert!(!bin.join("lampo.exe").exists());
    }

    #[test]
    fn uninstall_deletes_the_copies_and_the_emptied_folder_and_is_idempotent() {
        let (d, exe, bin) = setup();
        let (installer, cli) = with_cli(&exe, d.path());
        installer.install(&bin).unwrap();
        installer.uninstall(&bin).unwrap();
        assert!(!bin.exists());
        assert!(!installer.is_installed(&bin).unwrap());
        installer.uninstall(&bin).unwrap();
        assert!(exe.is_file() && cli.is_file(), "the shipped executables are never touched");
    }

    /// The CLI resolves the site through the shim index (`lampo exec`, `lampo env`, "Apri
    /// terminale"): it belongs to the app, not to the shims, and survives `shims uninstall`.
    #[test]
    fn uninstall_keeps_the_shim_index_and_unknown_files() {
        let (d, exe, bin) = setup();
        let (installer, _cli) = with_cli(&exe, d.path());
        installer.install(&bin).unwrap();
        std::fs::write(bin.join("shim-index.json"), b"{}").unwrap();
        std::fs::write(bin.join("php.old-42"), b"shim-v0").unwrap(); // moved aside by a refresh
        std::fs::write(bin.join("lampo.old-7"), b"cli-v0").unwrap();
        std::fs::write(bin.join("notes.txt"), b"mine").unwrap();
        installer.uninstall(&bin).unwrap();
        let mut left: Vec<_> = std::fs::read_dir(&bin).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["notes.txt", "shim-index.json"]);
        assert_eq!(std::fs::read(bin.join("shim-index.json")).unwrap(), b"{}");
    }

    #[test]
    fn only_copies_moved_aside_by_a_refresh_count_as_leftovers() {
        let targets = [PathBuf::from(r"C:\bin\php.exe"), PathBuf::from(r"C:\bin\lampo.exe")];
        assert_eq!(moved_aside_path(&targets[0]).extension().unwrap().to_str().unwrap(), format!("old-{}", std::process::id()));
        for leftover in ["php.old-1234", "PHP.old-1", "lampo.old-99"] {
            assert!(is_moved_aside(leftover, &targets), "{leftover}");
        }
        for other in ["php.exe", "php.old-", "php.old-12a", "phpx.old-1", "shim-index.json", "old-1", "composer.old-1"] {
            assert!(!is_moved_aside(other, &targets), "{other}");
        }
    }

    #[test]
    fn locked_copy_hint_tells_when_the_running_cli_is_the_copy_itself() {
        let (_d, _exe, bin) = setup();
        std::fs::create_dir_all(&bin).unwrap();
        assert!(locked_copy_hint(&bin).starts_with("is a shim"));
        let own_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        assert!(locked_copy_hint(&own_dir).contains("install folder"));
    }

    #[test]
    fn missing_shim_binary_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let installer = ExeShimInstaller::new(dir.path().join("nope.exe"), vec!["php"]);
        assert!(matches!(installer.install(&dir.path().join("bin")), Err(PortError::NotFound { .. })));
    }
}
