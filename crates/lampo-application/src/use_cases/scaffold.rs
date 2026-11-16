//! Setting up the folder of a new project: `git clone` or the Laravel Installer (managed by
//! Lampo in its own tools folder, run with Lampo's PHP and Composer). Runs inside the app as a
//! background task; the front-end then detects and creates the project on the new folder.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lampo_domain::package::EntryRole;
use lampo_domain::paths::LampoPaths;
use lampo_domain::php::{ExtensionName, PhpMinor};
use lampo_domain::ports::{CommandSpec, ExecutableLocator, InstalledPackageRegistry, PhpIniWriter, ProgressSink, ScaffoldFiles, StreamingCommandRunner};
use lampo_domain::progress::{CancelFlag, Progress, ProgressStage};
use lampo_domain::project::{ProjectId, ProjectRoot};
use lampo_domain::scaffold::{self, FolderName, GitBranch, LaravelOptions, PathState, RepositoryUrl, StarterKit, TestingFramework};
use lampo_domain::service::LogStream;
use lampo_domain::site::SiteId;
use lampo_domain::web::PhpIniSpec;

use super::generation::{base_ini, bundled};
use super::support::{composer_phar_for, installed_php_minors, package_missing, php_package, validation};
use crate::dto::*;
use crate::error::{AppError, AppResult, ErrorCode};

/// Lines of output quoted in the error of a failed command.
const ERROR_TAIL_LINES: usize = 6;

pub struct ScaffoldProjectDeps {
    pub files: Arc<dyn ScaffoldFiles>,
    pub runner: Arc<dyn StreamingCommandRunner>,
    pub locator: Arc<dyn ExecutableLocator>,
    pub installed: Arc<dyn InstalledPackageRegistry>,
    pub php_ini: Arc<dyn PhpIniWriter>,
    pub paths: LampoPaths,
}

pub struct ScaffoldProject {
    deps: ScaffoldProjectDeps,
}

/// What to create, validated.
enum Source {
    Git { url: RepositoryUrl, branch: Option<GitBranch> },
    Laravel { php: Option<PhpMinor>, options: LaravelOptions },
}

/// Step `n` of `total`, with the program's output as messages.
struct Step<'a> {
    progress: &'a dyn ProgressSink,
    subject: &'static str,
    done: u64,
    total: u64,
}

impl Step<'_> {
    fn say(&self, line: impl Into<String>) {
        self.progress.report(Progress::new(ProgressStage::Running, self.done, Some(self.total)).with_subject(self.subject).with_message(line));
    }
}

/// The `PackageMissing` error for git, recognisable by the GUI (`details.tool`).
fn git_missing() -> AppError {
    AppError::new(ErrorCode::PackageMissing, "Git not found: install it from git-scm.com").with_details(serde_json::json!({ "tool": "git" }))
}

impl ScaffoldProject {
    pub fn new(deps: ScaffoldProjectDeps) -> Self {
        Self { deps }
    }

    pub async fn execute(&self, request: ScaffoldProjectRequest, progress: &dyn ProgressSink, cancel: &CancelFlag) -> AppResult<ScaffoldProjectResultDto> {
        let (source, name) = parse(&request)?;
        let parent = ProjectRoot::new(&request.parent_dir)?;
        if !matches!(self.deps.files.path_state(parent.as_path()), PathState::EmptyDir | PathState::NonEmptyDir) {
            return Err(validation(format!("the folder `{parent}` does not exist")));
        }
        let destination = parent.as_path().join(name.as_str());
        let before = self.deps.files.path_state(&destination);
        scaffold::check_destination(&destination, before, matches!(source, Source::Git { .. }))?;

        let result = match &source {
            Source::Git { url, branch } => self.clone(url, branch.as_ref(), parent.as_path(), &destination, progress, cancel).await.map(|()| None),
            Source::Laravel { php, options } => self.laravel(*php, options, &name, parent.as_path(), &destination, progress, cancel).await.map(Some),
        };
        match result {
            Ok(php) => Ok(ScaffoldProjectResultDto { folder: destination.display().to_string(), php_version: php.map(|p| p.to_string()) }),
            Err(e) => {
                // Only what this run created: an empty folder the user picked stays.
                if before == PathState::Missing && self.deps.files.path_state(&destination) != PathState::Missing {
                    if let Err(cleanup) = self.deps.files.remove_dir_all(&destination) {
                        tracing::warn!("could not remove the partial folder {}: {cleanup}", destination.display());
                    }
                }
                Err(e)
            }
        }
    }

    fn git_exe(&self) -> AppResult<PathBuf> {
        self.deps.locator.find_all("git.exe")?.into_iter().next().ok_or_else(git_missing)
    }

    async fn clone(&self, url: &RepositoryUrl, branch: Option<&GitBranch>, parent: &Path, destination: &Path, progress: &dyn ProgressSink, cancel: &CancelFlag) -> AppResult<()> {
        let git = self.git_exe()?;
        let spec = CommandSpec {
            program: git,
            args: scaffold::git_clone_args(url, branch, destination),
            working_dir: Some(parent.to_path_buf()),
            env: scaffold::git_env(),
            path_prepend: Vec::new(),
        };
        let step = Step { progress, subject: "git clone", done: 0, total: 1 };
        self.run(&step, "git clone", spec, cancel).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn laravel(
        &self,
        php: Option<PhpMinor>,
        options: &LaravelOptions,
        name: &FolderName,
        parent: &Path,
        destination: &Path,
        progress: &dyn ProgressSink,
        cancel: &CancelFlag,
    ) -> AppResult<PhpMinor> {
        let installed = self.deps.installed.list()?;
        let php = match php {
            Some(p) => p,
            None => *installed_php_minors(&installed).last().ok_or_else(|| package_missing("PHP"))?,
        };
        if php < PhpMinor::LARAVEL_INSTALLER_MIN {
            return Err(validation(format!("the Laravel Installer needs PHP {} or later (PHP {php} was chosen)", PhpMinor::LARAVEL_INSTALLER_MIN)));
        }
        let package = php_package(&installed, php).ok_or_else(|| package_missing(format!("PHP {php}")))?;
        let php_exe = package.entry(EntryRole::Cli).ok_or_else(|| package_missing(format!("the PHP {php} command line")))?.to_path_buf();
        let php_dir = package.install_dir.clone();
        let phar = composer_phar_for(&installed, php).ok_or_else(|| package_missing("Composer"))?;
        let git_dir = match self.git_exe() {
            Ok(git) => git.parent().map(Path::to_path_buf),
            Err(e) if options.git => return Err(e),
            Err(_) => None,
        };

        // Lampo's own tool files: php.ini with what Composer and Laravel need, `composer.bat`.
        let ini_dir = self.deps.paths.tool_php_ini_dir(php);
        self.deps.php_ini.write(&PhpIniSpec {
            site: SiteId::Project(ProjectId::new("lampo-tools").expect("valid slug")),
            php,
            php_dir: php_dir.clone(),
            output_dir: ini_dir.clone(),
            extensions: scaffold::TOOL_PHP_EXTENSIONS.iter().map(|n| bundled(&ExtensionName::new(n).expect("valid extension name"))).collect(),
            values: base_ini(),
            error_log: self.deps.paths.cli_php_error_log(),
        })?;
        let composer_dir = self.deps.paths.tool_composer_dir(php);
        self.deps.files.write_file(&composer_dir.join("composer.bat"), &scaffold::composer_wrapper_script(&php_exe, &phar))?;
        let env = scaffold::tool_env(php, &php_dir, &composer_dir, &ini_dir, &git_dir.into_iter().collect::<Vec<_>>());
        let spec = |args: Vec<String>, cwd: &Path| CommandSpec {
            program: php_exe.clone(),
            args,
            working_dir: Some(cwd.to_path_buf()),
            env: env.vars.clone(),
            path_prepend: env.path_prepend.clone(),
        };

        // The installer, in Lampo's tools folder (never the user's global Composer).
        let installer_dir = self.deps.paths.laravel_installer_dir(php);
        let script = scaffold::laravel_installer_script(&installer_dir);
        let step = Step { progress, subject: "laravel-installer", done: 0, total: 2 };
        if self.deps.files.is_file(&script) {
            let update = self.run(&step, "composer update (Laravel Installer)", spec(scaffold::installer_update_args(&phar, &installer_dir), &installer_dir), cancel).await;
            match update {
                Ok(()) => {}
                Err(e) if e.code == ErrorCode::Cancelled => return Err(e),
                Err(e) => step.say(format!("Could not update the Laravel Installer, continuing with the installed version ({})", e.message)),
            }
        } else {
            self.deps.files.create_dir_all(&installer_dir)?;
            self.run(&step, "composer require laravel/installer", spec(scaffold::installer_require_args(&phar, &installer_dir), &installer_dir), cancel).await?;
            if !self.deps.files.is_file(&script) {
                return Err(AppError::new(ErrorCode::ProcessFailed, format!("the Laravel Installer was not installed: {} is missing", script.display())));
            }
        }

        let step = Step { progress, subject: "laravel new", done: 1, total: 2 };
        self.run(&step, "laravel new", spec(scaffold::laravel_new_args(&script, name, options), parent), cancel).await?;
        if self.deps.files.path_state(destination) == PathState::Missing {
            return Err(AppError::new(ErrorCode::ProcessFailed, format!("the Laravel Installer did not create {}", destination.display())));
        }
        Ok(php)
    }

    /// Runs `spec`, streaming its cleaned output lines as progress messages. A non-zero exit is
    /// `ProcessFailed` quoting the last lines.
    async fn run(&self, step: &Step<'_>, label: &str, spec: CommandSpec, cancel: &CancelFlag) -> AppResult<()> {
        step.say(format!("> {label}"));
        let tail = Mutex::new(VecDeque::with_capacity(ERROR_TAIL_LINES));
        let on_line = |_stream: LogStream, line: &str| {
            if let Some(clean) = scaffold::clean_output_line(line) {
                let mut t = tail.lock().unwrap_or_else(|p| p.into_inner());
                if t.len() == ERROR_TAIL_LINES {
                    t.pop_front();
                }
                t.push_back(clean.clone());
                drop(t);
                step.say(clean);
            }
        };
        let code = self.deps.runner.run_streaming(spec, &on_line, cancel).await?;
        if code == Some(0) {
            return Ok(());
        }
        let tail: Vec<String> = tail.into_inner().unwrap_or_else(|p| p.into_inner()).into();
        let exit = code.map(|c| format!("exit code {c}")).unwrap_or_else(|| "unknown exit code".into());
        let mut message = format!("{label} failed ({exit})");
        if !tail.is_empty() {
            message.push_str(": ");
            message.push_str(&tail.join(" | "));
        }
        // `details.output` is a string for every error code (`config-rejected` too): one line each.
        Err(AppError::new(ErrorCode::ProcessFailed, message).with_details(serde_json::json!({ "output": tail.join("\n") })))
    }
}

/// Request → validated source and folder name.
fn parse(request: &ScaffoldProjectRequest) -> AppResult<(Source, FolderName)> {
    let explicit = request.folder_name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    match &request.source {
        ScaffoldSourceDto::GitClone { url, branch } => {
            let url = RepositoryUrl::new(url)?;
            let branch = branch.as_deref().map(str::trim).filter(|b| !b.is_empty()).map(GitBranch::new).transpose()?;
            let name = match explicit {
                Some(n) => FolderName::new(n)?,
                None => scaffold::folder_name_from_repo_url(url.as_str()).ok_or_else(|| validation(format!("cannot derive a folder name from `{}`: give one", url.as_str())))?,
            };
            Ok((Source::Git { url, branch }, name))
        }
        ScaffoldSourceDto::Laravel { php_version, starter_kit, testing, git, npm } => {
            let name = FolderName::new(explicit.ok_or_else(|| validation("the project name is required"))?)?;
            scaffold::validate_laravel_name(&name)?;
            let php = php_version.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(PhpMinor::parse).transpose()?;
            let starter_kit = match starter_kit {
                StarterKitDto::None => StarterKit::None,
                StarterKitDto::React => StarterKit::React,
                StarterKitDto::Vue => StarterKit::Vue,
                StarterKitDto::Livewire => StarterKit::Livewire,
                StarterKitDto::Svelte => StarterKit::Svelte,
            };
            let testing = match testing {
                TestingFrameworkDto::Pest => TestingFramework::Pest,
                TestingFrameworkDto::Phpunit => TestingFramework::PhpUnit,
            };
            Ok((Source::Laravel { php, options: LaravelOptions { starter_kit, testing, git: *git, npm: *npm } }, name))
        }
    }
}
