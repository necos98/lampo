//! The modules compiled into a PHP build (`bcmath`, `ctype`, `zip` in some builds…): they have
//! no `ext\php_<name>.dll`, only the build knows them. `php -n -m` (no php.ini, so no extension
//! is loaded) prints exactly those, under `[PHP Modules]`.
//!
//! Running PHP costs a process, so the answer is cached per `php.exe` for the life of the process
//! and asked again only when that file changes (a patch update replaces the slot's files).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, SystemTime};

use lampo_domain::php::ExtensionName;
use lampo_domain::{PortError, PortResult};

/// How long `php -n -m` may take: a healthy build answers in well under a second.
const LIST_TIMEOUT: Duration = Duration::from_secs(15);
/// More output than any build's module list (a few hundred bytes): stop reading past it.
const MAX_OUTPUT_BYTES: u64 = 64 * 1024;
const MODULES_HEADER: &str = "[PHP Modules]";
/// Modules `php -m` lists by a display name instead of their php.ini name (PHP 8.5 compiles
/// OPcache in).
const DISPLAY_NAMES: &[(&str, &str)] = &[("Zend OPcache", "opcache")];

/// `php.exe` as it was when its modules were listed: a different size or time is another build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BuildStamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl BuildStamp {
    fn of(php_exe: &Path) -> PortResult<Self> {
        let meta = std::fs::metadata(php_exe).map_err(|e| PortError::io(format!("reading {}", php_exe.display()), e))?;
        Ok(Self { len: meta.len(), modified: meta.modified().ok() })
    }
}

/// The compiled-in modules of each `php.exe` asked so far.
#[derive(Debug, Default)]
pub(crate) struct BuiltinModules {
    cache: Mutex<HashMap<PathBuf, (BuildStamp, Vec<ExtensionName>)>>,
}

impl BuiltinModules {
    /// The modules compiled into `php_exe`, sorted, from the cache while the file is unchanged.
    pub(crate) fn of(&self, php_exe: &Path) -> PortResult<Vec<ExtensionName>> {
        let stamp = BuildStamp::of(php_exe)?;
        if let Some((cached, modules)) = self.lock().get(php_exe)
            && *cached == stamp
        {
            return Ok(modules.clone());
        }
        // Not under the lock: another build may be asked meanwhile, and asking twice is harmless.
        let modules = parse_modules(&list_modules(php_exe)?);
        self.lock().insert(php_exe.to_path_buf(), (stamp, modules.clone()));
        Ok(modules)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, (BuildStamp, Vec<ExtensionName>)>> {
        self.cache.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The names under `[PHP Modules]` (the `[Zend Modules]` that follow are not extensions one
/// enables by name), sorted and unique. Lines that are not extension names are skipped.
fn parse_modules(output: &str) -> Vec<ExtensionName> {
    let section = output.lines().map(str::trim).skip_while(|line| *line != MODULES_HEADER).skip(1).take_while(|line| !line.starts_with('['));
    let ini_name = |line: &str| DISPLAY_NAMES.iter().find(|(display, _)| *display == line).map_or(line, |(_, name)| *name).to_string();
    let mut modules: Vec<ExtensionName> = section.filter_map(|line| ExtensionName::new(ini_name(line)).ok()).collect();
    modules.sort();
    modules.dedup();
    modules
}

/// Runs `php_exe -n -m` (no console window) and returns its standard output. `Process` when it
/// does not start, fails, prints too much or outlives [`LIST_TIMEOUT`] (it is killed).
fn list_modules(php_exe: &Path) -> PortResult<String> {
    let program = php_exe.display().to_string();
    let mut command = Command::new(php_exe);
    command.args(["-n", "-m"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    hide_console(&mut command);
    let mut child = command.spawn().map_err(|e| PortError::process(&program, e))?;
    let mut stdout = child.stdout.take().ok_or_else(|| PortError::process(&program, "no standard output"))?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let read = stdout.by_ref().take(MAX_OUTPUT_BYTES + 1).read_to_end(&mut out).map(|_| out);
        let _ = sender.send(read);
    });
    let read = receiver.recv_timeout(LIST_TIMEOUT);
    let complete = matches!(&read, Ok(Ok(out)) if out.len() as u64 <= MAX_OUTPUT_BYTES);
    if !complete {
        // Timed out, unreadable or too much output: the process may still be running.
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| PortError::process(&program, e))?;
    let out = match read {
        Ok(Ok(out)) if complete => out,
        Ok(Ok(_)) => return Err(PortError::process(&program, format!("printed more than {MAX_OUTPUT_BYTES} bytes"))),
        Ok(Err(e)) => return Err(PortError::process(&program, e)),
        Err(_) => return Err(PortError::process(&program, format!("did not answer within {} s", LIST_TIMEOUT.as_secs()))),
    };
    if !status.success() {
        return Err(PortError::process(&program, format!("`-n -m` exited with {status}")));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "[PHP Modules]\r\nbcmath\r\ncalendar\r\nCore\r\nReflection\r\nSPL\r\nzlib\r\nbcmath\r\n\r\n[Zend Modules]\r\nZend OPcache\r\n";

    fn names(list: &[ExtensionName]) -> Vec<&str> {
        list.iter().map(ExtensionName::as_str).collect()
    }

    #[test]
    fn reads_the_php_modules_only() {
        assert_eq!(names(&parse_modules(OUTPUT)), ["bcmath", "calendar", "core", "reflection", "spl", "zlib"]);
        assert!(parse_modules("").is_empty());
        assert!(parse_modules("PHP Warning: something\n").is_empty());
        assert_eq!(names(&parse_modules("noise\n[PHP Modules]\nctype\nnot a name\n")), ["ctype"]);
        // PHP 8.5 compiles OPcache in and lists it by its display name.
        assert_eq!(names(&parse_modules("[PHP Modules]\nuri\nZend OPcache\nzlib\n\n[Zend Modules]\nZend OPcache\n")), ["opcache", "uri", "zlib"]);
    }

    /// A stand-in for `php.exe` printing `body` and counting its runs in `runs.txt`.
    #[cfg(windows)]
    fn fake_php(dir: &Path, body: &str) -> PathBuf {
        let script = dir.join("php.cmd");
        let runs = dir.join("runs.txt");
        let echo: String = body.lines().map(|line| if line.is_empty() { "echo.\r\n".to_string() } else { format!("echo {line}\r\n") }).collect();
        std::fs::write(&script, format!("@echo off\r\necho run>>\"{}\"\r\n{echo}", runs.display())).unwrap();
        script
    }

    #[cfg(windows)]
    fn runs(dir: &Path) -> usize {
        std::fs::read_to_string(dir.join("runs.txt")).map(|t| t.lines().count()).unwrap_or(0)
    }

    #[cfg(windows)]
    #[test]
    fn asks_each_build_once_until_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let php = fake_php(dir.path(), "[PHP Modules]\nbcmath\nzip\n\n[Zend Modules]\n");
        let modules = BuiltinModules::default();
        assert_eq!(names(&modules.of(&php).unwrap()), ["bcmath", "zip"]);
        assert_eq!(names(&modules.of(&php).unwrap()), ["bcmath", "zip"]);
        assert_eq!(runs(dir.path()), 1);

        fake_php(dir.path(), "[PHP Modules]\nbcmath\n");
        assert_eq!(names(&modules.of(&php).unwrap()), ["bcmath"], "a replaced php.exe is asked again");
        assert_eq!(runs(dir.path()), 2);
    }

    #[cfg(windows)]
    #[test]
    fn a_failing_or_missing_build_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let failing = dir.path().join("php.cmd");
        std::fs::write(&failing, "@echo off\r\nexit /b 3\r\n").unwrap();
        let modules = BuiltinModules::default();
        assert!(matches!(modules.of(&failing), Err(PortError::Process { .. })));
        assert!(modules.of(&dir.path().join("missing.exe")).is_err());
    }
}
