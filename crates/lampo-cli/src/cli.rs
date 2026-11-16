//! Command-line surface (clap). Non-interactive by design: no prompts ever; destructive commands
//! require an explicit flag (`--yes`, `--drop-database`); nothing triggers UAC unless
//! `--allow-elevation` is passed. Every command except `exec` supports `--json` (stable DTOs
//! from `lampo_application::dto`).

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};

use crate::exit_code::ExitCode;

const ABOUT: &str = "Lampo: local PHP environments for Windows (CLI for coding agents and terminals)";

const LONG_ABOUT: &str = "\
Lampo manages a local PHP environment: one Apache with a virtual host per project, any PHP \
version, shared MySQL per version, optional Redis, and a separate environment per git worktree.

NEW HERE (AI agents): run `lampo agent context --json` first. It prints everything about the \
current folder in one call: site URL, PHP, database connection, services, log files, the available \
commands, exit codes and hints.

Design rules:
  * Never prompts and never blocks waiting for input.
  * `--json` prints exactly one JSON document on stdout and nothing else there; diagnostics go to \
stderr. Errors in JSON mode are a JSON document too: {\"error\": {\"code\", \"message\", \"hint\", \
\"exitCode\"}}.
  * Never triggers a UAC prompt unless `--allow-elevation` is given (exit code 7 otherwise).
  * Read-only commands run in-process (the app need not be running). Commands that change state \
go to the Lampo app, which is started headless when needed (disable with `--no-start`).
  * The site is resolved from --project, then LAMPO_PROJECT (only inside LAMPO_PROJECT_ROOT or \
outside every project), then the current folder (including git worktrees), then the global \
default PHP.";

#[derive(Debug, Parser)]
#[command(name = "lampo", version, about = ABOUT, long_about = LONG_ABOUT, propagate_version = true, arg_required_else_help = true)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(subcommand)]
    pub command: Command,
}

/// The clap command with the generated exit-code table appended to `--help`.
pub fn build_command() -> clap::Command {
    Cli::command().after_long_help(exit_codes_help()).after_help(exit_codes_help())
}

/// The exit-code table, rendered for `--help` from [`ExitCode::TABLE`].
pub fn exit_codes_help() -> String {
    let mut text = String::from("Exit codes:\n");
    for (code, meaning) in ExitCode::TABLE {
        text.push_str(&format!("  {:>2}  {meaning}\n", code.code()));
    }
    text.push_str("\nOther codes: `lampo exec` returns the child's own exit code.");
    text
}

#[derive(Debug, Clone, Args)]
pub struct GlobalArgs {
    /// Machine-readable output: exactly one JSON document on stdout (errors too, as
    /// `{"error": {"code", "message", "hint", "exitCode"}}`). Ignored by `exec`, which passes the
    /// child's output through.
    #[arg(long, global = true)]
    pub json: bool,
    /// Site to act on (`shop`, `shop@feature-x`); overrides `LAMPO_PROJECT` and the current folder.
    #[arg(long, short = 'p', global = true, value_name = "SITE")]
    pub project: Option<String>,
    /// Allow a UAC prompt (hosts-file update). Without it such commands exit with code 7.
    #[arg(long, global = true)]
    pub allow_elevation: bool,
    /// Do not start the app automatically for commands that need it (exit code 9 instead).
    #[arg(long, global = true)]
    pub no_start: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Services and projects overview (works without the app; service states are then `unknown`).
    Status,
    /// Manage projects (a project is a folder served at its own URL).
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Per-git-worktree environments (own URL and optionally own database).
    #[command(subcommand)]
    Worktree(WorktreeCommand),
    /// Start services (all planned services when none given).
    Up(ServicesArgs),
    /// Stop services (all when none given).
    Down(ServicesArgs),
    /// Restart services (all running ones when none given).
    Restart(ServicesArgs),
    /// Stop every service and exit the Lampo app (never starts it; a no-op when not running).
    Quit,
    /// Read service and site logs; `--follow` streams until Ctrl+C.
    Logs(LogsArgs),
    /// Downloadable packages (PHP, MySQL, Redis, extensions...).
    #[command(subcommand)]
    Package(PackageCommand),
    /// Databases on the resolved site's MySQL instance (goes through the app).
    #[command(subcommand)]
    Db(DbCommand),
    /// Run a command with the site's toolchain (PHP, Composer, MySQL client) on PATH.
    ///
    /// Everything after `--` is the command line. The child's stdio is inherited and its exit
    /// code is returned. Ctrl+C goes to the child. `--json` is ignored (the output is the
    /// child's).
    ///
    /// Example: lampo exec -- composer install --no-interaction
    Exec(ExecArgs),
    /// Print the site's toolchain environment (PATH additions, PHPRC, LAMPO_* variables).
    ///
    /// With `--shell` the output is a script fragment to evaluate in that shell:
    ///   PowerShell: lampo env --shell powershell | Out-String | Invoke-Expression
    ///   Git Bash:   eval "$(lampo env --shell bash)"
    ///   cmd:        lampo env --shell cmd > %TEMP%\lampo-env.cmd & call %TEMP%\lampo-env.cmd
    Env(EnvArgs),
    /// Diagnose the installation (VC++ runtimes, ports, PATH shadowing, hosts file...). Exit 14
    /// when problems are found.
    Doctor,
    /// Commands designed for coding agents.
    #[command(subcommand)]
    Agent(AgentCommand),
    /// Install the php/composer/mysql/mysqldump/redis-cli shims (and a copy of `lampo`) and add
    /// their folder to the user PATH (or remove them).
    ///
    /// Inside a project the `mysql` shim selects the site's database unless the command names
    /// one (`-D <db>`, `--database=<db>`, a database as the last argument); `--database=` with
    /// nothing after `=` connects without one. While the site's database does not exist yet the
    /// shim connects without it and says so in one line on stderr. `mysqldump` dumps the site's
    /// database when the command names nothing to dump.
    #[command(subcommand)]
    Shims(ShimsCommand),
    /// Lampo's global settings: ports (Apache, MySQL instances, Redis, Mailpit), default PHP, loopback
    /// port range. Without a subcommand: `settings get`.
    Settings {
        #[command(subcommand)]
        command: Option<SettingsCommand>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// List projects.
    List,
    /// Show one project (default: the project resolved from the current folder).
    Info {
        /// Project id (`shop`).
        id: Option<String>,
    },
    /// Show what Lampo detects in a folder, WITHOUT registering it (read-only, no app needed).
    ///
    /// Reports the project type (laravel, wordpress, symfony, composer or none), the preset and
    /// PHP minor `project create` would pick, the suggested name and id, the evidence found, the
    /// services the folder's own files show it uses (Docker Compose: mysql, redis), warnings
    /// about what Lampo cannot provide (PostgreSQL, Mailpit...) and the id of the project
    /// already registered for the folder, if any. Nothing is written. Run it before
    /// `project create` and pass `--preset` / `--php` there when a detected default is wrong.
    ///
    /// Example: lampo project detect C:\laragon\www\shop --json
    Detect {
        /// Folder to analyse (default: current directory; relative paths resolve against it).
        path: Option<String>,
    },
    /// Register a folder (default: current folder). Defaults come from detection and presets.
    ///
    /// Addressing comes from `--addressing`, else the `default-addressing` setting (built-in:
    /// `no-elevation`, `<id>.localhost` plus a dedicated loopback port, which never needs
    /// elevation). `hosts-only` (just `<id>.<tld>`) and `standard` (`<id>.<tld>` plus
    /// `<id>.localhost`) write the hosts file and need `--allow-elevation`.
    ///
    /// Example: lampo project create C:\sites\shop --addressing hosts-only --allow-elevation
    Create(ProjectCreateArgs),
    /// Set up a NEW project folder, then register it: clone a git repository (`--git <URL>`) or
    /// create a Laravel application with the Laravel Installer (`--laravel <NAME>`).
    ///
    /// The installer is managed by Lampo (its own tools folder, Lampo's PHP and Composer; the
    /// global Composer is never touched). git must be on PATH. The tools' output is streamed to
    /// stderr. On failure or Ctrl+C the partially created folder is removed.
    ///
    /// Examples:
    ///   lampo project new --git https://github.com/acme/shop.git --in D:\sites
    ///   lampo project new --laravel shop --starter-kit react --php 8.4 --activate
    New(ProjectNewArgs),
    /// Serve the project.
    Activate {
        /// Project id (default: resolved from the current folder).
        id: Option<String>,
    },
    /// Stop serving the project.
    Deactivate {
        /// Project id (default: resolved from the current folder).
        id: Option<String>,
    },
    /// Switch the project's PHP minor (e.g. `8.3`). Same as `lampo project set --php`
    ///
    /// The version comes first, then the optional project id.
    ///
    /// Example: lampo project set-php 8.4 shop
    SetPhp {
        /// PHP minor version, `8.3`.
        #[arg(value_name = "VERSION")]
        php_version: String,
        /// Project id (default: resolved from the current folder).
        id: Option<String>,
    },
    /// Change the project's PHP version, extensions, add-ons (Xdebug), php.ini values, services (MySQL/Redis), database name, public folder, Apache directives, display name or addresses
    ///
    /// Only what is named changes, in one atomic edit of the stored project (no lost update
    /// with the GUI); then Apache, php.ini, services and the hosts file are regenerated as after
    /// a GUI save. Every flag is repeatable and list flags also take comma-separated values;
    /// removals apply before additions; adding something already there (or removing something
    /// absent) is a no-op. Worktree environments follow their parent project. Prints the
    /// updated project (`--json`: the ProjectDto).
    ///
    /// Examples:
    ///   lampo project set --php 8.4 --ext intl,gd
    ///   lampo project set --ext intl,gd --ini memory_limit=512M
    ///   lampo project set --addon xdebug --ini xdebug.mode=debug,develop
    ///   lampo project set --no-addon xdebug --no-ini xdebug.mode
    ///   lampo project set --service redis --service mysql@5.7 --database shop_legacy
    ///   lampo project set shop --public-dir public --json
    ///   lampo project set --address api.shop.localhost --address loopback
    ///   lampo project set --address shop.test --allow-elevation
    ///   lampo project set --no-address 127.0.0.1:8101 --name "Shop Legacy"
    #[command(verbatim_doc_comment)]
    Set(Box<ProjectSetArgs>),
    /// Change the project's display name (the id, URLs, folder and database stay)
    ///
    /// Same as `lampo project set --name`. Quote names with spaces.
    ///
    /// Example: lampo project rename "Shop Legacy" shop
    Rename {
        /// New display name (1-80 characters).
        name: String,
        /// Project id (default: resolved from the current folder or --project).
        id: Option<String>,
    },
    /// Unregister a project from Lampo. Requires `--yes` (never prompts)
    ///
    /// Removes its configuration and its worktree environments; the folder, the files and the
    /// databases are NEVER deleted (drop a database with `lampo db drop` first if wanted). An
    /// active project is removed too: Lampo just stops serving it, as in the app. A project with
    /// a hosts-file name (`shop.test`) needs `--allow-elevation` (exit 7, nothing removed,
    /// otherwise). Undo: `lampo project create <folder>` registers the folder again (with new
    /// defaults).
    ///
    /// Example: lampo project remove shop --yes
    Remove {
        /// Project id (default: resolved from the current folder or --project).
        id: Option<String>,
        /// Confirm the removal.
        #[arg(long)]
        yes: bool,
    },
    /// Register a project exported from Lampo (a `.lampo.toml` file, e.g. from another machine)
    ///
    /// The imported project is inactive and uses HTTP. The file may come from someone else, so
    /// what could run code on this machine is NEVER imported: its custom Apache directives and
    /// the php.ini keys auto_prepend_file, auto_append_file, extension, zend_extension,
    /// extension_dir, open_basedir, disable_functions and include_path. They are listed in the
    /// output (`--json`: `droppedApacheDirectives`, `droppedPhpIniKeys`) with a warning on
    /// stderr: review them, then add back what you trust with `lampo project set`. An id
    /// already in use gets a numeric suffix and new addresses. A hosts-file name (`shop.test`)
    /// needs `--allow-elevation` (exit 7, nothing imported, otherwise).
    ///
    /// Example: lampo project import D:\exports\shop.lampo.toml --root C:\sites\shop
    Import {
        /// The exported file (relative paths resolve against the current directory).
        file: String,
        /// The project's folder on this machine, when it is not the exported one.
        #[arg(long)]
        root: Option<String>,
    },
}

#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("change").required(true).multiple(true).args([
    "php", "ext", "no_ext", "addon", "no_addon", "ini", "no_ini", "service", "no_service", "database", "public_dir", "apache_directives",
    "name", "address", "no_address",
])))]
pub struct ProjectSetArgs {
    /// Project id (default: resolved from the current folder or --project).
    pub id: Option<String>,
    /// Switch to another PHP minor (`8.4`), as `lampo project set-php` does. It must be
    /// installed (`lampo package install php 8.4`; exit 10 otherwise), and so must the
    /// project's add-ons for it. Applied first: the other flags are checked against it.
    #[arg(long, value_name = "VERSION")]
    pub php: Option<String>,
    /// Enable a bundled PHP extension (`intl`, `gd`, `pdo_mysql`, `opcache`...; `php_intl.dll`
    /// also works). Must ship with the project's PHP (the error lists the available ones).
    #[arg(long, value_name = "NAME", value_delimiter = ',')]
    pub ext: Vec<String>,
    /// Disable a bundled PHP extension.
    #[arg(long, value_name = "NAME", value_delimiter = ',')]
    pub no_ext: Vec<String>,
    /// Enable an add-on extension package: `xdebug`, `php-redis`. It must be installed for the
    /// project's PHP first (`lampo package install xdebug --php 8.3`; exit 10 otherwise).
    #[arg(long, value_name = "KIND", value_delimiter = ',')]
    pub addon: Vec<String>,
    /// Disable an add-on extension (the package stays installed).
    #[arg(long, value_name = "KIND", value_delimiter = ',')]
    pub no_addon: Vec<String>,
    /// Set a php.ini override, `KEY=VALUE` (`memory_limit=512M`, `xdebug.mode=debug`). The value
    /// is taken verbatim, commas included; an empty value (`display_errors=`) is kept as such.
    #[arg(long, value_name = "KEY=VALUE")]
    pub ini: Vec<String>,
    /// Remove a php.ini override (back to Lampo's default for that key).
    #[arg(long, value_name = "KEY", value_delimiter = ',')]
    pub no_ini: Vec<String>,
    /// Use a service: `redis`, `mysql` (the default MySQL instance), `mysql@5.7` or `mysql-5.7`
    /// (a configured series; switches series if MySQL is already used). Adding MySQL also
    /// assigns the project's database name; create the database with `lampo db create`.
    #[arg(long, value_name = "KIND[@VERSION]", value_delimiter = ',')]
    pub service: Vec<String>,
    /// Stop using a service kind (`redis`, `mysql`). Databases are never dropped.
    #[arg(long, value_name = "KIND", value_delimiter = ',')]
    pub no_service: Vec<String>,
    /// Database name shown in the connection details and the LAMPO_DB_* variables (the project
    /// must use MySQL). Does not create, rename or copy any database.
    #[arg(long, value_name = "NAME")]
    pub database: Option<String>,
    /// Document root relative to the project folder (`public`, `web`); `""` = the folder itself.
    #[arg(long, value_name = "DIR")]
    pub public_dir: Option<String>,
    /// Extra Apache directives inside the project's <VirtualHost> (replaces the current ones;
    /// `""` clears them). From a file in PowerShell: --apache-directives (Get-Content x.conf -Raw)
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub apache_directives: Option<String>,
    /// New display name (the id, URLs, folder and database stay).
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    /// Add a site address, appended after the current ones (the first address stays the
    /// primary one): `<name>.localhost` (no UAC), `127.0.0.1:<port>` or `loopback` (a free
    /// dedicated port; no-op if the project has one), or a hosts-file name such as `shop.test`
    /// (needs --allow-elevation: exit 7, nothing changed, otherwise). A name or port used by
    /// another site is a conflict (exit 5).
    #[arg(long, value_name = "ADDRESS", value_delimiter = ',')]
    pub address: Vec<String>,
    /// Remove a site address (same forms; `loopback` = the dedicated port, whichever it is).
    /// At least one must remain; removing the first makes the next one primary. Removing a
    /// hosts-file name also needs --allow-elevation.
    #[arg(long, value_name = "ADDRESS", value_delimiter = ',')]
    pub no_address: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ProjectCreateArgs {
    /// Project folder (default: current directory).
    pub path: Option<String>,
    /// Display name (default: the folder name).
    #[arg(long)]
    pub name: Option<String>,
    /// Project id used in URLs (default: derived from the name).
    #[arg(long)]
    pub id: Option<String>,
    /// Preset id (`laravel`, `wordpress`, `generic-php`...; default: detected).
    #[arg(long)]
    pub preset: Option<String>,
    /// PHP minor (`8.3`).
    #[arg(long)]
    pub php: Option<String>,
    /// Which addresses the project gets (default: the `default-addressing` setting).
    /// `hosts-only` and `standard` need `--allow-elevation`.
    #[arg(long, value_enum, value_name = "MODE")]
    pub addressing: Option<AddressingArg>,
    /// Same as `--addressing standard` (kept for compatibility).
    #[arg(long, conflicts_with = "addressing")]
    pub hosts_name: bool,
    /// Serve the project right away.
    #[arg(long)]
    pub activate: bool,
    /// Do not create the project's MySQL database (by default it is created, starting the
    /// MySQL instance if needed; an existing database is kept).
    #[arg(long)]
    pub no_database: bool,
}

#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("source").required(true).args(["git", "laravel"])))]
pub struct ProjectNewArgs {
    /// Repository to clone (https or ssh URL).
    #[arg(long, value_name = "URL")]
    pub git: Option<String>,
    /// Branch or tag to clone (with --git; default: the remote's default branch).
    #[arg(long, conflicts_with = "laravel")]
    pub branch: Option<String>,
    /// Folder name of the clone (with --git; default: the repository name).
    #[arg(long, conflicts_with = "laravel")]
    pub folder: Option<String>,
    /// Create a Laravel application named NAME (also its folder name).
    #[arg(long, value_name = "NAME")]
    pub laravel: Option<String>,
    /// Folder the new project folder is created in (default: current directory).
    #[arg(long = "in", value_name = "DIR")]
    pub parent: Option<String>,
    /// PHP minor (`8.4`): runs the Laravel Installer and is used by the project (default: the
    /// newest installed PHP for Laravel, detection for a clone).
    #[arg(long)]
    pub php: Option<String>,
    /// Laravel starter kit.
    #[arg(long, value_enum, default_value_t = StarterKitArg::None, conflicts_with = "git")]
    pub starter_kit: StarterKitArg,
    /// Laravel: PHPUnit instead of Pest.
    #[arg(long, conflicts_with = "git")]
    pub phpunit: bool,
    /// Laravel: initialise a git repository with a first commit (needs git).
    #[arg(long, conflicts_with = "git")]
    pub init_git: bool,
    /// Laravel: install and build the front-end with npm (needs Node.js).
    #[arg(long, conflicts_with = "git")]
    pub npm: bool,
    /// Only set up the folder; do not register it as a project.
    #[arg(long)]
    pub no_register: bool,
    /// Which addresses the project gets (default: the `default-addressing` setting).
    /// `hosts-only` and `standard` need `--allow-elevation`, checked before the folder is set up.
    #[arg(long, value_enum, value_name = "MODE", conflicts_with = "no_register")]
    pub addressing: Option<AddressingArg>,
    /// Serve the project right away.
    #[arg(long)]
    pub activate: bool,
    /// Do not create the project's MySQL database.
    #[arg(long)]
    pub no_database: bool,
}

/// `--addressing` / `--default-addressing`: the addresses a new project gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AddressingArg {
    /// Only `<id>.<tld>` (hosts file; needs --allow-elevation).
    HostsOnly,
    /// `<id>.<tld>` (hosts file; needs --allow-elevation) plus `<id>.localhost`.
    Standard,
    /// `<id>.localhost` plus a dedicated `127.0.0.1` port: never needs elevation.
    NoElevation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StarterKitArg {
    None,
    React,
    Vue,
    Livewire,
    Svelte,
}

#[derive(Debug, Subcommand)]
pub enum WorktreeCommand {
    /// Create (or return) the environment of the worktree at PATH (default: current folder).
    /// Idempotent: safe to run on every agent start.
    Up(WorktreeUpArgs),
    /// Remove the environment of PATH or SITE (default: current folder).
    Down(WorktreeDownArgs),
    /// List environments (all, or those of the project given with the global `--project`).
    List,
    /// Remove environments whose folder no longer exists.
    Prune(WorktreePruneArgs),
}

#[derive(Debug, Args)]
pub struct WorktreeUpArgs {
    /// Any folder inside the worktree (default: current directory).
    pub path: Option<String>,
    /// Parent project id (default: from the worktree's main repository).
    #[arg(long)]
    pub parent: Option<String>,
    /// Database: `shared` (default, the parent's), `fresh` (empty own schema), or
    /// `clone:<database>` (own schema copied from that database).
    #[arg(long, default_value = "shared", value_name = "MODE")]
    pub db: String,
}

#[derive(Debug, Args)]
pub struct WorktreeDownArgs {
    /// Worktree folder or site id such as `shop@feature-x` (default: current directory).
    pub target: Option<String>,
    /// Also drop the environment's own database.
    #[arg(long)]
    pub drop_database: bool,
}

#[derive(Debug, Args)]
pub struct WorktreePruneArgs {
    /// Only report what would be removed.
    #[arg(long)]
    pub dry_run: bool,
    /// Also drop the databases owned by the removed environments.
    #[arg(long)]
    pub drop_databases: bool,
}

#[derive(Debug, Args)]
pub struct ServicesArgs {
    /// Services: `apache`, `redis`, `mysql-8.4`, ... (default: all).
    pub services: Vec<String>,
}

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// `apache`, `apache-error`, `mysql-8.4`, `mysql-8.4-error`, `redis`, `access`, `error`,
    /// `php` (the last three for the resolved site, or `access:<site>` ...). Default: list
    /// the available sources.
    pub source: Option<String>,
    /// Number of last lines to print.
    #[arg(long, short = 'n', default_value_t = 100)]
    pub tail: usize,
    /// Only lines since: `30s`, `10m`, `2h`, `1d` or a Unix timestamp in milliseconds.
    #[arg(long)]
    pub since: Option<String>,
    /// Keep printing new lines until Ctrl+C (JSON: one `LogLineDto` per line, NDJSON).
    #[arg(long, short = 'f')]
    pub follow: bool,
}

#[derive(Debug, Subcommand)]
pub enum PackageCommand {
    /// List the catalog, or only what is installed with `--installed`.
    List {
        /// Filter by kind: `php`, `mysql`, `redis`, `xdebug`...
        #[arg(long)]
        kind: Option<String>,
        /// Only installed packages (with the sites using them).
        #[arg(long)]
        installed: bool,
    },
    /// Install `<kind> [<version|series>]`, e.g. `php 8.3`, `mysql 8.4`, `xdebug --php 8.3`.
    ///
    /// A value with up to two numeric parts (`8.3`) is a series (latest patch); three or more
    /// (`8.3.12`) is an exact version. Progress goes to stderr.
    Install {
        /// Package kind.
        kind: String,
        /// Series (`8.3`) or exact version (`8.3.12`); default: latest.
        #[arg(value_name = "VERSION")]
        package_version: Option<String>,
        /// For add-on extensions: the PHP minor they are built for.
        #[arg(long)]
        php: Option<String>,
    },
    /// Uninstall `<kind> [<version|series>]`, e.g. `php 8.2`, `mysql 5.7`, `xdebug --php 8.3`. Requires `--yes`
    ///
    /// Same syntax as `install`: `8.2` is a series, `8.2.27` an exact version, `--php` picks the
    /// add-on build for that PHP minor. Exactly one installed build must match (several: exit 4
    /// with `details.candidates`; none: exit 3). Refused while it is the last build something
    /// uses: a project or worktree environment (PHP minor, add-on, MySQL series), the default
    /// PHP or an always-on MySQL (exit 5, `details.usedBy`). Before the files are deleted, the
    /// sites and services using the build move to the one that remains (e.g. 8.3.14 when 8.3.15
    /// goes), restarting Apache and those services; the command waits until the files are gone.
    /// Files still locked by another program fail it and keep the package installed. Projects
    /// and databases are never touched. Prints the removed package (`--json`: its id).
    ///
    /// Examples:
    ///   lampo package uninstall php 8.2 --yes
    ///   lampo package uninstall xdebug --php 8.3 --yes
    ///   lampo package uninstall mysql 5.7.44 --yes --json
    #[command(verbatim_doc_comment)]
    Uninstall {
        /// Package kind.
        kind: String,
        /// Series (`8.3`) or exact version (`8.3.12`); default: the only installed build.
        #[arg(value_name = "VERSION")]
        package_version: Option<String>,
        /// For add-on extensions: the PHP minor they are built for.
        #[arg(long)]
        php: Option<String>,
        /// Confirm the removal (lampo never prompts).
        #[arg(long)]
        yes: bool,
    },
    /// List the bundled extensions of a PHP build: what `lampo project set --ext` accepts
    ///
    /// Default PHP: the resolved project's minor (worktree environments follow their project),
    /// else the global default PHP. Read-only, no app needed. With a resolved project on the
    /// same minor, each extension shows whether the project enables it. Add-ons (`xdebug`,
    /// `php-redis`) are packages, not listed here: see `lampo package list --installed`.
    ///
    /// Examples:
    ///   lampo package extensions
    ///   lampo package extensions --php 8.4 --json
    #[command(verbatim_doc_comment)]
    Extensions {
        /// PHP minor to list (`8.3`); must be installed (exit 10 otherwise).
        #[arg(long)]
        php: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum DbCommand {
    /// List databases on the site's MySQL instance.
    List,
    /// Create a database (default name: the site's database).
    Create {
        /// Database name.
        name: Option<String>,
    },
    /// Drop a database. Requires `--yes` (never prompts).
    Drop {
        /// Database name.
        name: String,
        /// Confirm the destructive action.
        #[arg(long)]
        yes: bool,
    },
    /// Import an SQL dump (default database: the site's).
    ///
    /// Plain `.sql`, gzip (`.sql.gz`, `.gz`) or a `.zip` holding exactly one `.sql` file, detected
    /// from the content and decompressed on the fly (never extracted to disk). Progress counts
    /// the bytes of the file as given.
    ///
    /// A dump whose first 64 KB contain `USE` or `CREATE DATABASE` (`mysqldump --databases`) is
    /// refused: its statements would run in the database it names, not in the target. If the
    /// import fails or is cancelled part-way, the target may be left incomplete.
    ///
    /// A dump made by MySQL 8.0 or MariaDB may need `--compat` to load: the file is never
    /// changed, the fixes apply to the importing session and to the stream on its way. When the
    /// header of such a dump names its server and the import fails, the error suggests the flag.
    Import {
        /// `.sql`, `.sql.gz` or `.zip` file.
        file: String,
        /// Target database.
        name: Option<String>,
        /// Import even a dump with `USE` / `CREATE DATABASE` statements (it may write to other
        /// databases than the target).
        #[arg(long)]
        allow_use: bool,
        /// The server the dump comes from (`lampo db import dump.sql --compat mariadb`).
        #[arg(long, value_enum, value_name = "SERVER")]
        compat: Option<DumpCompatArg>,
    },
    /// Export a database to an SQL file (default database: the site's).
    ///
    /// A file name ending in `.gz` (`backup.sql.gz`) is written gzip-compressed.
    Export {
        /// `.sql` or `.sql.gz` file to write.
        file: String,
        /// Source database.
        name: Option<String>,
    },
}

/// `lampo db import --compat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DumpCompatArg {
    /// MySQL 8.0: foreign keys on non-unique keys allowed (`restrict_fk_on_non_standard_key=OFF`
    /// on 8.4+), `DEFINER=` clauses of views, triggers and routines removed (the objects belong
    /// to the importing user).
    Mysql80,
    /// MariaDB: as mysql80, plus the first-line sandbox comment dropped and `utf8mb4_general_ci`
    /// as the default `utf8mb4` collation (session and a database created by the import).
    Mariadb,
}

#[derive(Debug, Args)]
pub struct ExecArgs {
    /// Command and arguments, after `--`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true, value_name = "COMMAND")]
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ShellKind {
    Powershell,
    Cmd,
    Bash,
}

#[derive(Debug, Args)]
pub struct EnvArgs {
    /// Print as a script fragment for this shell instead of a table.
    #[arg(long, value_enum, conflicts_with = "json")]
    pub shell: Option<ShellKind>,
}

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Everything an agent needs about the current project/worktree, in one call.
    ///
    /// Includes the resolved site, URL, PHP, database connection, services, log files, available
    /// commands, the exit-code table, the JSON error shape and actionable hints. Works when the
    /// app is not running. Use `--json` for the machine-readable form.
    Context(AgentContextArgs),
}

#[derive(Debug, Args)]
pub struct AgentContextArgs {
    /// Claude Code `SessionStart` hook mode: reads the hook JSON (its `cwd`) from stdin and, inside
    /// a Lampo project or worktree, prints a compact summary as `additionalContext`. Prints nothing
    /// and exits 0 elsewhere or on any error.
    #[arg(long)]
    pub hook: bool,
}

#[derive(Debug, Subcommand)]
pub enum ShimsCommand {
    /// Whether the shims are installed, on PATH, and not shadowed.
    Status,
    /// Install the shims and a copy of `lampo`, and add their folder to the user PATH (no
    /// elevation needed).
    Install,
    /// Remove the shim folder from the user PATH and delete it (shims, the `lampo` copy, the shim
    /// index). The app and its own `lampo.exe` are untouched; `shims install` puts them back.
    Uninstall,
}

#[derive(Debug, Subcommand)]
pub enum SettingsCommand {
    /// Show the settings (read-only, works without the app). `--json`: the SettingsDto.
    Get,
    /// Change ports, the default PHP, the loopback port range or the default addressing
    ///
    /// Only what is named changes, in one atomic edit of the stored settings inside the app (no
    /// lost update with the GUI). Ports are checked first: 1-65535, not used by another Lampo
    /// service or by a site's dedicated loopback port (exit 5), not taken by another program
    /// (exit 5, port-in-use). Running services whose port changed are restarted on the new one,
    /// as after a GUI save; stopped ones use it when they next start. Projects' own files are
    /// not edited: update DB_PORT / REDIS_PORT / MAIL_PORT / APP_URL yourself (the output lists
    /// the sites). Mailpit has two ports: SMTP (`--port mailpit=PORT`) and its web UI
    /// (`--ui-port mailpit=PORT`).
    /// MySQL instances (series) cannot be added or removed: only their ports change.
    ///
    /// Examples:
    ///   lampo settings set --apache-port 8080
    ///   lampo settings set --mysql-port 8.4=3310 --mysql-port 5.7=3311 --redis-port 6380
    ///   lampo settings set --port mailpit=2025 --ui-port mailpit=9025
    ///   lampo settings set --default-php 8.4 --json
    ///   lampo settings set --loopback-range 8100-8299
    ///   lampo settings set --default-addressing hosts-only
    #[command(verbatim_doc_comment)]
    Set(SettingsSetArgs),
}

#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("change").required(true).multiple(true).args(["apache_port", "mysql_port", "redis_port", "port", "ui_port", "default_php", "loopback_range", "default_addressing"])))]
pub struct SettingsSetArgs {
    /// Apache's shared port (host-name URLs use it; loopback-port URLs keep their own port).
    #[arg(long, value_name = "PORT", value_parser = clap::value_parser!(u16).range(1..))]
    pub apache_port: Option<u16>,
    /// Port of a configured MySQL instance: `SERIES=PORT` (`8.4=3310`); repeatable or
    /// comma-separated.
    #[arg(long, value_name = "SERIES=PORT", value_delimiter = ',')]
    pub mysql_port: Vec<String>,
    /// Redis port.
    #[arg(long, value_name = "PORT", value_parser = clap::value_parser!(u16).range(1..))]
    pub redis_port: Option<u16>,
    /// Port of any configured service instance by id: `SERVICE=PORT` (`mysql-8.4=3310`,
    /// `redis=6380`, `mailpit=1025` for Mailpit's SMTP); repeatable or comma-separated.
    #[arg(long, value_name = "SERVICE=PORT", value_delimiter = ',')]
    pub port: Vec<String>,
    /// Web UI port of an instance that has one: `SERVICE=PORT` (`mailpit=8025`); repeatable
    /// or comma-separated.
    #[arg(long, value_name = "SERVICE=PORT", value_delimiter = ',')]
    pub ui_port: Vec<String>,
    /// PHP minor the shims use outside any project (`8.4`); must be installed.
    #[arg(long, value_name = "MINOR")]
    pub default_php: Option<String>,
    /// Range new dedicated loopback ports (worktree environments) come from: `START-END`
    /// (start >= 1024). Existing sites keep their ports.
    #[arg(long, value_name = "START-END")]
    pub loopback_range: Option<String>,
    /// Addresses `project create` and `project new` give a project when `--addressing` is not
    /// passed. Existing projects keep theirs.
    #[arg(long, value_enum, value_name = "MODE")]
    pub default_addressing: Option<AddressingArg>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_definition_is_consistent() {
        build_command().debug_assert();
    }

    #[test]
    fn help_lists_every_exit_code() {
        let help = exit_codes_help();
        for (code, _) in ExitCode::TABLE {
            assert!(help.contains(&format!("{:>2}  ", code.code())));
        }
    }

    #[test]
    fn exec_keeps_hyphenated_arguments() {
        let cli = Cli::try_parse_from(["lampo", "exec", "--", "php", "-r", "echo 1;", "--version"]).unwrap();
        let Command::Exec(args) = cli.command else { panic!("not exec") };
        assert_eq!(args.command, ["php", "-r", "echo 1;", "--version"]);
    }

    #[test]
    fn global_flags_work_after_the_subcommand() {
        let cli = Cli::try_parse_from(["lampo", "project", "list", "--json", "-p", "shop"]).unwrap();
        assert!(cli.global.json);
        assert_eq!(cli.global.project.as_deref(), Some("shop"));
    }

    #[test]
    fn env_shell_conflicts_with_json() {
        assert!(Cli::try_parse_from(["lampo", "env", "--shell", "bash", "--json"]).is_err());
        assert!(Cli::try_parse_from(["lampo", "env", "--shell", "cmd"]).is_ok());
    }

    #[test]
    fn rejects_unknown_shell_and_missing_command() {
        assert!(Cli::try_parse_from(["lampo", "env", "--shell", "fish"]).is_err());
        assert!(Cli::try_parse_from(["lampo", "exec"]).is_err());
        assert!(Cli::try_parse_from(["lampo"]).is_err());
    }
}
