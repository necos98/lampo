//! Setting up the folder of a *new* project before it is registered: cloning a git repository or
//! creating a Laravel application with the Laravel Installer. Pure rules only: input validation,
//! folder names, command lines and the environment of the tools. The application use case
//! `ScaffoldProject` runs the commands through the `StreamingCommandRunner` port.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::DomainError;
use crate::php::PhpMinor;
use crate::shim::ToolchainEnv;

fn invalid(message: impl Into<String>) -> DomainError {
    DomainError::InvalidScaffold(message.into())
}

// ---------------------------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------------------------

const RESERVED_NAMES: &[&str] =
    &["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
const FORBIDDEN_CHARS: &str = "<>:\"/\\|?*";

/// A single folder name that Windows accepts (no path separators, no reserved device names, no
/// trailing dot or space). Surrounding whitespace is trimmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderName(String);

impl FolderName {
    pub fn new(raw: &str) -> Result<Self, DomainError> {
        let name = raw.trim();
        if name.is_empty() {
            return Err(invalid("the folder name is empty"));
        }
        if name.chars().count() > 255 {
            return Err(invalid("the folder name is longer than 255 characters"));
        }
        if name == "." || name == ".." {
            return Err(invalid(format!("`{name}` is not a folder name")));
        }
        if let Some(c) = name.chars().find(|c| c.is_control() || FORBIDDEN_CHARS.contains(*c)) {
            return Err(invalid(format!("the folder name `{name}` contains `{}`, which Windows does not allow", c.escape_default())));
        }
        if name.ends_with('.') {
            return Err(invalid(format!("the folder name `{name}` cannot end with a dot")));
        }
        let stem = name.split('.').next().unwrap_or(name).trim_end().to_ascii_uppercase();
        if RESERVED_NAMES.contains(&stem.as_str()) {
            return Err(invalid(format!("`{name}` is a reserved name on Windows")));
        }
        Ok(Self(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FolderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The Laravel Installer's own rule for application names (letters, digits, `-`, `_`, `.`),
/// which also keeps the name safe inside the shell command lines the installer builds. A leading
/// `-` would be read as an option.
pub fn validate_laravel_name(name: &FolderName) -> Result<(), DomainError> {
    let s = name.as_str();
    if s.starts_with('-') || !s.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        return Err(invalid(format!("the Laravel project name `{s}` may only contain letters, digits, dashes, underscores and dots (and cannot start with a dash)")));
    }
    Ok(())
}

/// The folder name a clone of `url` gets by default (`https://github.com/acme/shop.git`,
/// `git@github.com:acme/shop.git` → `shop`), like `git clone` itself.
pub fn folder_name_from_repo_url(url: &str) -> Option<FolderName> {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\', ':']).next()?;
    let last = last.strip_suffix(".git").unwrap_or(last);
    FolderName::new(last).ok()
}

// ---------------------------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------------------------

/// A repository URL handed to `git clone` (https, ssh, scp-like `git@host:path`, a local path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryUrl(String);

impl RepositoryUrl {
    pub fn new(raw: &str) -> Result<Self, DomainError> {
        let url = raw.trim();
        if url.is_empty() {
            return Err(invalid("the repository URL is empty"));
        }
        if url.starts_with('-') || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(invalid(format!("`{url}` is not a repository URL")));
        }
        Ok(Self(url.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A branch (or tag) name for `git clone --branch`, checked with the essential
/// `git check-ref-format` rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitBranch(String);

impl GitBranch {
    pub fn new(raw: &str) -> Result<Self, DomainError> {
        let b = raw.trim();
        let bad = b.is_empty()
            || b.starts_with(['-', '/', '.'])
            || b.ends_with(['/', '.'])
            || b.ends_with(".lock")
            || b.contains("..")
            || b.contains("@{")
            || b.contains("//")
            || b.chars().any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c));
        if bad {
            return Err(invalid(format!("`{b}` is not a valid branch name")));
        }
        Ok(Self(b.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `git clone --progress [--branch <b>] -- <url> <dest>` (arguments after the program).
pub fn git_clone_args(url: &RepositoryUrl, branch: Option<&GitBranch>, destination: &Path) -> Vec<String> {
    let mut args = vec!["clone".to_string(), "--progress".to_string()];
    if let Some(b) = branch {
        args.extend(["--branch".to_string(), b.as_str().to_string()]);
    }
    args.extend(["--".to_string(), url.as_str().to_string(), destination.display().to_string()]);
    args
}

/// Never wait for a console credential prompt (there is no console): fail instead. Git
/// Credential Manager may still show its own window.
pub fn git_env() -> Vec<(String, String)> {
    vec![("GIT_TERMINAL_PROMPT".into(), "0".into())]
}

// ---------------------------------------------------------------------------------------------
// Laravel
// ---------------------------------------------------------------------------------------------

/// Starter kit flags of `laravel new` (`Svelte` since installer 5.x).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StarterKit {
    #[default]
    None,
    React,
    Vue,
    Livewire,
    Svelte,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TestingFramework {
    #[default]
    Pest,
    PhpUnit,
}

/// Options of `laravel new`. With `--no-interaction` the installer asks nothing and defaults
/// nothing, so every choice is passed explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LaravelOptions {
    pub starter_kit: StarterKit,
    pub testing: TestingFramework,
    /// `--git`: initialise a repository with a first commit (needs git).
    pub git: bool,
    /// `--npm` (install and build the front-end, needs Node.js) or `--no-node`.
    pub npm: bool,
}

/// The Composer package Lampo installs into its own tools folder.
pub const LARAVEL_INSTALLER_PACKAGE: &str = "laravel/installer";

/// The installer's entry script inside its tools folder.
pub fn laravel_installer_script(installer_dir: &Path) -> PathBuf {
    installer_dir.join("vendor").join("laravel").join("installer").join("bin").join("laravel")
}

/// `php <installer> new <name> --no-interaction ...` (arguments after the PHP executable).
pub fn laravel_new_args(installer_script: &Path, name: &FolderName, options: &LaravelOptions) -> Vec<String> {
    let mut args = vec![installer_script.display().to_string(), "new".into(), name.as_str().into(), "--no-interaction".into()];
    let kit = match options.starter_kit {
        StarterKit::None => None,
        StarterKit::React => Some("--react"),
        StarterKit::Vue => Some("--vue"),
        StarterKit::Livewire => Some("--livewire"),
        StarterKit::Svelte => Some("--svelte"),
    };
    args.extend(kit.map(String::from));
    args.push(match options.testing {
        TestingFramework::Pest => "--pest".into(),
        TestingFramework::PhpUnit => "--phpunit".into(),
    });
    if options.git {
        args.push("--git".into());
    }
    args.push(if options.npm { "--npm".into() } else { "--no-node".into() });
    // Laravel Boost (AI tooling) is opt-in elsewhere; keep the scaffold minimal and quick.
    args.push("--no-boost".into());
    args
}

/// `php composer.phar require laravel/installer` in the tools folder.
pub fn installer_require_args(composer_phar: &Path, installer_dir: &Path) -> Vec<String> {
    composer_args(composer_phar, installer_dir, &["require", LARAVEL_INSTALLER_PACKAGE])
}

/// `php composer.phar update` in the tools folder (keeps the installer current).
pub fn installer_update_args(composer_phar: &Path, installer_dir: &Path) -> Vec<String> {
    composer_args(composer_phar, installer_dir, &["update"])
}

fn composer_args(composer_phar: &Path, working_dir: &Path, command: &[&str]) -> Vec<String> {
    let mut args = vec![composer_phar.display().to_string()];
    args.extend(command.iter().map(|s| s.to_string()));
    args.extend(["--no-interaction".to_string(), "--no-ansi".to_string(), format!("--working-dir={}", working_dir.display())]);
    args
}

/// Extensions of the php.ini used by Lampo's own tools: what Composer and a new Laravel
/// application need (`zip` for dist downloads, SQLite for the installer's default database).
pub const TOOL_PHP_EXTENSIONS: &[&str] = &["curl", "fileinfo", "mbstring", "openssl", "pdo_mysql", "mysqli", "zip", "pdo_sqlite", "sqlite3"];

/// `composer.bat`: lets `composer ...` command lines (the installer spawns them through the
/// shell) run Lampo's Composer with the chosen PHP.
pub fn composer_wrapper_script(php_exe: &Path, composer_phar: &Path) -> String {
    let esc = |p: &Path| p.display().to_string().replace('%', "%%");
    format!("@echo off\r\n\"{}\" \"{}\" %*\r\n", esc(php_exe), esc(composer_phar))
}

/// Environment of the scaffolding commands: the `composer.bat` folder and the chosen PHP first on
/// `PATH` (so nested `php`/`composer` calls use them, not the cwd-resolving shims), then
/// `extra_dirs` (git). Lampo's tool php.ini via `PHPRC`; Composer and git never prompt.
pub fn tool_env(php: PhpMinor, php_dir: &Path, composer_dir: &Path, phprc_dir: &Path, extra_dirs: &[PathBuf]) -> ToolchainEnv {
    let mut path_prepend = vec![composer_dir.to_path_buf(), php_dir.to_path_buf()];
    path_prepend.extend(extra_dirs.iter().cloned());
    let mut vars = vec![("PHPRC".to_string(), phprc_dir.display().to_string()), ("LAMPO_PHP_VERSION".to_string(), php.to_string()), ("COMPOSER_NO_INTERACTION".to_string(), "1".to_string())];
    vars.extend(git_env());
    ToolchainEnv { path_prepend, vars }
}

// ---------------------------------------------------------------------------------------------
// Destination and output
// ---------------------------------------------------------------------------------------------

/// What is at a destination path before scaffolding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathState {
    Missing,
    EmptyDir,
    NonEmptyDir,
    File,
}

/// The destination must not exist; `git clone` also accepts an empty folder.
pub fn check_destination(destination: &Path, state: PathState, allow_empty_dir: bool) -> Result<(), DomainError> {
    match state {
        PathState::Missing => Ok(()),
        PathState::EmptyDir if allow_empty_dir => Ok(()),
        PathState::EmptyDir | PathState::NonEmptyDir => Err(invalid(format!("the folder `{}` already exists", destination.display()))),
        PathState::File => Err(invalid(format!("`{}` already exists and is a file", destination.display()))),
    }
}

/// One line of tool output for the user: ANSI escape sequences removed, trailing whitespace
/// trimmed; `None` when nothing visible is left.
pub fn clean_output_line(line: &str) -> Option<String> {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            if !c.is_control() || c == '\t' {
                out.push(c);
            }
            continue;
        }
        match chars.peek() {
            // CSI: parameters, then one final byte in @..~
            Some('[') => {
                chars.next();
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            // OSC: up to BEL or ESC \
            Some(']') => {
                chars.next();
                while let Some(n) = chars.next() {
                    if n == '\u{7}' {
                        break;
                    }
                    if n == '\u{1b}' {
                        chars.next();
                        break;
                    }
                }
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    let trimmed = out.trim_end();
    (!trimmed.trim_start().is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &str) -> FolderName {
        FolderName::new(s).unwrap()
    }

    #[test]
    fn folder_names_follow_windows_rules() {
        assert_eq!(name("  shop ").as_str(), "shop");
        assert_eq!(name("my shop.v2").as_str(), "my shop.v2");
        for bad in ["", "  ", ".", "..", "a/b", "a\\b", "c:", "x?", "a*", "q\"", "<x>", "pipe|", "dot.", "CON", "con", "nul.txt", "COM1", "lpt9.tar.gz", "a\u{1}b"] {
            assert!(FolderName::new(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(FolderName::new("console").is_ok());
        assert!(FolderName::new("COM10").is_ok());
        assert!(FolderName::new(&"x".repeat(256)).is_err());
    }

    #[test]
    fn laravel_names_are_restricted_to_the_installer_rule() {
        assert!(validate_laravel_name(&name("shop-app_2.0")).is_ok());
        assert!(validate_laravel_name(&name("negozio-è")).is_ok());
        assert!(validate_laravel_name(&name("my shop")).is_err());
        assert!(validate_laravel_name(&name("-shop")).is_err());
        assert!(validate_laravel_name(&name("shop&x")).is_err());
    }

    #[test]
    fn folder_name_is_derived_from_the_repository_url() {
        let derived = |u: &str| folder_name_from_repo_url(u).map(|n| n.to_string());
        assert_eq!(derived("https://github.com/acme/shop.git").as_deref(), Some("shop"));
        assert_eq!(derived("https://github.com/acme/shop").as_deref(), Some("shop"));
        assert_eq!(derived("https://github.com/acme/shop.git/").as_deref(), Some("shop"));
        assert_eq!(derived("git@github.com:acme/shop.git").as_deref(), Some("shop"));
        assert_eq!(derived("git@github.com:shop.git").as_deref(), Some("shop"));
        assert_eq!(derived("ssh://git@host:2222/team/my.site.git").as_deref(), Some("my.site"));
        assert_eq!(derived("D:\\repos\\blog").as_deref(), Some("blog"));
        assert_eq!(derived(""), None);
        assert_eq!(derived("https://host/.git"), None);
    }

    #[test]
    fn repository_urls_and_branches_are_validated() {
        assert!(RepositoryUrl::new(" https://github.com/acme/shop.git ").is_ok());
        assert!(RepositoryUrl::new("git@github.com:acme/shop.git").is_ok());
        for bad in ["", "   ", "--upload-pack=evil", "https://x/a b"] {
            assert!(RepositoryUrl::new(bad).is_err(), "{bad:?}");
        }
        assert!(GitBranch::new("feature/login").is_ok());
        assert!(GitBranch::new("v1.2.0").is_ok());
        for bad in ["", "-x", "a..b", "a b", "x.lock", "a/", "/a", "a~1", "a^", "a:b", "x@{1}", "a\\b"] {
            assert!(GitBranch::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn git_clone_arguments() {
        let url = RepositoryUrl::new("https://github.com/acme/shop.git").unwrap();
        let dest = Path::new("C:\\sites\\shop");
        assert_eq!(git_clone_args(&url, None, dest), ["clone", "--progress", "--", "https://github.com/acme/shop.git", "C:\\sites\\shop"]);
        let branch = GitBranch::new("develop").unwrap();
        assert_eq!(git_clone_args(&url, Some(&branch), dest), ["clone", "--progress", "--branch", "develop", "--", "https://github.com/acme/shop.git", "C:\\sites\\shop"]);
        assert_eq!(git_env(), [("GIT_TERMINAL_PROMPT".to_string(), "0".to_string())]);
    }

    #[test]
    fn laravel_arguments_for_each_option() {
        let script = Path::new("C:\\tools\\laravel");
        let args = |o: LaravelOptions| laravel_new_args(script, &name("shop"), &o)[1..].join(" ");
        assert_eq!(args(LaravelOptions::default()), "new shop --no-interaction --pest --no-node --no-boost");
        let all = LaravelOptions { starter_kit: StarterKit::React, testing: TestingFramework::PhpUnit, git: true, npm: true };
        assert_eq!(args(all), "new shop --no-interaction --react --phpunit --git --npm --no-boost");
        for (kit, flag) in [(StarterKit::Vue, "--vue"), (StarterKit::Livewire, "--livewire"), (StarterKit::Svelte, "--svelte")] {
            let a = args(LaravelOptions { starter_kit: kit, ..Default::default() });
            assert_eq!(a, format!("new shop --no-interaction {flag} --pest --no-node --no-boost"));
        }
        assert_eq!(laravel_new_args(script, &name("shop"), &LaravelOptions::default())[0], "C:\\tools\\laravel");
    }

    #[test]
    fn installer_commands_and_script() {
        let phar = Path::new("C:\\composer\\composer.phar");
        let dir = Path::new("C:\\data\\tools\\laravel-installer\\8.4");
        assert_eq!(
            installer_require_args(phar, dir),
            ["C:\\composer\\composer.phar", "require", "laravel/installer", "--no-interaction", "--no-ansi", "--working-dir=C:\\data\\tools\\laravel-installer\\8.4"]
        );
        assert_eq!(installer_update_args(phar, dir)[1], "update");
        assert_eq!(laravel_installer_script(dir), Path::new("C:\\data\\tools\\laravel-installer\\8.4\\vendor\\laravel\\installer\\bin\\laravel"));
        assert_eq!(composer_wrapper_script(Path::new("C:\\php\\php.exe"), Path::new("C:\\100%\\composer.phar")), "@echo off\r\n\"C:\\php\\php.exe\" \"C:\\100%%\\composer.phar\" %*\r\n");
    }

    #[test]
    fn tool_environment_puts_composer_and_php_first() {
        let php = PhpMinor::parse("8.3").unwrap();
        let env = tool_env(php, Path::new("C:\\php"), Path::new("C:\\tools\\composer\\8.3"), Path::new("C:\\ini"), &[PathBuf::from("C:\\git\\cmd")]);
        assert_eq!(env.path_prepend, [PathBuf::from("C:\\tools\\composer\\8.3"), PathBuf::from("C:\\php"), PathBuf::from("C:\\git\\cmd")]);
        assert!(env.vars.contains(&("PHPRC".into(), "C:\\ini".into())));
        assert!(env.vars.contains(&("GIT_TERMINAL_PROMPT".into(), "0".into())));
        assert!(env.vars.contains(&("COMPOSER_NO_INTERACTION".into(), "1".into())));
    }

    #[test]
    fn destination_rules() {
        let p = Path::new("C:\\sites\\shop");
        assert!(check_destination(p, PathState::Missing, false).is_ok());
        assert!(check_destination(p, PathState::EmptyDir, true).is_ok());
        assert!(check_destination(p, PathState::EmptyDir, false).is_err());
        assert!(check_destination(p, PathState::NonEmptyDir, true).is_err());
        assert!(check_destination(p, PathState::File, true).is_err());
    }

    #[test]
    fn output_lines_are_cleaned() {
        assert_eq!(clean_output_line("\u{1b}[32m  INFO \u{1b}[39m Done.  ").as_deref(), Some("  INFO  Done."));
        assert_eq!(clean_output_line("\u{1b}]8;;https://x\u{7}link\u{1b}]8;;\u{7}").as_deref(), Some("link"));
        assert_eq!(clean_output_line("Receiving objects:  45% (9/20)").as_deref(), Some("Receiving objects:  45% (9/20)"));
        assert_eq!(clean_output_line("   "), None);
        assert_eq!(clean_output_line("\u{1b}[2K"), None);
    }
}
