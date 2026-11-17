# Lampo: architecture

A lightweight local PHP development environment for Windows: one Apache (mod_fcgid) with a
virtual host per project, any PHP from 5.6 up, shared MySQL instances per version, optional Redis
and Mailpit (a shared SMTP catcher with a web UI), per-git-worktree environments for parallel
coding agents. Rust core + Tauri 2 + SvelteKit GUI,
plus an agent-friendly `lampo` CLI and `.exe` shims for `php`, `composer`, `mysql`, `mysqldump`
and `redis-cli`.

## Architecture: ports and adapters

```
            ┌────────────── composition roots (only places naming adapters) ──────────────┐
            │  apps/desktop/src-tauri (GUI, tray, control-pipe server)                     │
            │  crates/lampo-cli: `lampo` (src/composition.rs), `lampo-shim` (bin main)     │
            └──────────────┬──────────────────────────────────────────────────┬───────────┘
                           ▼                                                  ▼
                   lampo-application  (use cases, DTOs, events, control protocol)
                           ▼
                     lampo-domain  (entities, rules, port traits — no IO)
                           ▲ implemented by
   lampo-infra-store · lampo-infra-packages · lampo-infra-web · lampo-infra-services
```

| Path | Owns | Reason to change |
|------|------|------------------|
| `crates/lampo-domain` | Entities/value objects (project, site, worktree environment, PHP minor, package, service, preset, address, hosts block, shim index, settings, Claude Code plugin files), domain rules, `DomainError`/`PortError`, **every port trait** | Business rules |
| `crates/lampo-application` | Use cases (`Arc<dyn Port>` constructors), serde DTOs, front-end event names, CLI↔app control protocol, task registry, embedded Claude Code skills (`build.rs`) | Behaviour of features |
| `crates/lampo-infra-store` | TOML repositories (projects, worktrees, settings, presets), app paths, app location file, shim index file, shim installer (shims + CLI copy), project detectors, git metadata reader, user PATH, clock, generated agent-plugin files | Lampo's own files |
| `crates/lampo-infra-packages` | Embedded + windows.php.net catalogs, downloader, SHA-256, zip extraction, installed registry, VC++ runtime detection, post-install hooks | Where binaries come from |
| `crates/lampo-infra-web` | Apache config templates + `httpd -t` + runner (an httpd driver on the `ServiceHost` of `lampo-infra-services`), php.ini rendering, hosts-file block via the elevated `lampo-hosts-helper` | Apache/PHP/hosts specifics |
| `crates/lampo-infra-services` | Job Object supervisor, `ServiceHost` (start/restart/stop sequence shared by every service runner, Apache included), MySQL/Redis/Mailpit runners, port probe, log files, `DatabaseAdmin` (plain, `.sql.gz` and single-`.sql` `.zip` dumps, (de)compressed while streaming), MySQL root and project-account passwords (DPAPI), their generator, the account statements (`MysqlAccountAdmin`) and the option files handing passwords to the clients, control named pipe, headless app launcher, `claude` CLI runner, external desktop apps (PowerShell, Explorer, VS Code) | Local processes and IPC |
| `crates/lampo-cli` | `lampo` CLI and `lampo-shim` (copied as `php.exe` etc.): two composition roots (`src/composition.rs`, `src/bin/lampo-shim.rs`, which registers the shim tools) | CLI surface |
| `claude-plugin` | Claude Code plugin sources: `skills/<skill-id>/SKILL.md` + reference files and `icon.png` (embedded as is), trigger evals and a dev manifest (repository only) | What agents are taught |
| `apps/desktop` | SvelteKit GUI (`src/lib/api` typed API, `src/lib/messages` Italian strings) and `src-tauri` (Tauri commands, tray, composition root) | GUI |

Key rules: the domain does no IO and depends on no workspace crate; use cases see only ports;
traits are small and role-specific; new package kinds, presets, project detectors, resolvers,
shim tools, doctor checks and service kinds are added by implementing a trait and registering
it in the composition roots.

A **service kind** (MySQL, Redis, Mailpit, a future PostgreSQL) is data: a
`ServiceDescriptor` (default port, data dir, error log, client defaults, versioned or not,
optional second port and php.ini contributions; built-ins in `lampo_domain::service::builtin`)
plus a `ServiceRunner`. Projects list the services they use (`environment.services`), settings
list the configured instances (`services`); Apache stays special as the web tier. Files written
before this (`mysql`/`redis` fields, `mysql_instances`/`redis_port`, shim index v1) are migrated
on read; a settings file that lacks a built-in unversioned instance (Mailpit, added later)
gets it with its default ports, moved up past any port already taken.

**Two-port services.** An instance has its main `port` plus an optional `ui_port`
(`ServiceInstanceSettings`, written only when set) for kinds whose descriptor has a
`default_ui_port` (Mailpit: SMTP 1025, web UI 8025). `ServiceCatalog::ui_port` is the configured
value or the default; `ServiceLaunch::ports()` lists both, and every free-port check (sync,
`lampo up`, the runner, `doctor`), `Settings::validate` (distinct ports) and
`Settings::service_on_port` (loopback allocation) cover the UI port too. The settings patch
takes `serviceUiPorts` (`lampo settings set --ui-port mailpit=9025`; SMTP stays
`--port mailpit=1025`) and reports the change as `mailpit.ui-port`; the GUI edits it under the
instance's port. DTOs expose `uiPort`/`uiUrl` on service statuses and instances (omitted for
single-port kinds).

**Mailpit** (`MailpitRunner`, `crates/lampo-infra-services/src/mailpit.rs`) is one shared,
unversioned instance started on demand like Redis: `mailpit.exe --smtp 127.0.0.1:<port>
--listen 127.0.0.1:<ui_port> --database <data>\mailpit\data\mailpit.db
--smtp-auth-accept-any --smtp-auth-allow-insecure --smtp-disable-rdns --disable-version-check`,
output captured to `output.log` (`lampo logs mailpit`). Ready = SMTP `220` greeting and the UI
port accepting; stop = terminate (Mailpit has no shutdown command). Sites using it get
`SMTP = 127.0.0.1`, `smtp_port = <port>`, `sendmail_from = lampo@localhost` in their php.ini
(`ServiceDescriptor::php_ini`, merged between Lampo's base values and the project's own, which
win), a `mail` object (host, SMTP port, web URL) in `ProjectDto` and `agent context`, and
`LAMPO_MAIL_HOST`/`_PORT` from `lampo env` (the package folder goes on PATH like a client dir:
`mailpit sendmail`). The laravel, wordpress, symfony and generic-php presets include it; the
Docker detector maps `axllent/mailpit` and `mailhog/mailhog` (with a warning) to it.

Loopback ports are handed out by one `LoopbackPortAllocator` (shared by project creation and
`worktree up`), which also serialises every project save and removal with its conflict check.
`ProjectWriter` is the only way to save or remove a project (its fields are private): under that
lock it runs `ProjectEnvironmentPolicy` and `check_conflicts`, then saves. The policy is the
same for every front-end and path (GUI update, CLI patch, create, import, PHP switch): add-ons
installed for the project's PHP, bundled extensions present in its `ext` dir, MySQL series
configured in the settings, checked for what a save introduces (a PHP switch re-checks every
add-on; a bundled extension is checked when added to an existing project, since a build
without it only comments its php.ini line out), so a project left outdated by an uninstall can
still be renamed or switched off. Before the checks every save stores the extensions as the
project's PHP names them (`gd` ↔ `gd2`, the domain's alias table in `php::extension_alias`), so a
PHP switch renames them; one compiled into the build (`php -n -m`, cached per build by the
scanner) is accepted, reported in the save's `notices` and gets no php.ini line. A database name
without MySQL is refused, not dropped.
`DeleteProject` removes the project first and then its worktree environments, inside the lock,
so a concurrent edit never brings it back; it returns the worktree databases it leaves in MySQL
(`orphanedDatabases`). `lampo worktree prune` removes what it can and reports the rest in
`failed` (exit code of the first failure), publishing and syncing the removals all the same.

Every registry write (projects, worktree environments, settings) runs under that lock and takes
a commit number (`RegistryRevision`). A commit hands its snapshot, updated with what it saved
(`SiteSnapshot::with_project`…), to the sync (`EnvironmentSync::sync_sites`) and to the view it
returns, so a mutation reads each store once; the sync reads the files again when a later commit
has started since (`SiteSnapshot::is_current`). Store I/O called from async code runs off the
runtime's workers (`crate::blocking`: the blocking pool, or `block_in_place` for commits that
borrow). The first-run wizard installs without syncing (`InternalInstallRequest::skip_sync`)
and syncs once at the end.

The GUI saves a whole project (`UpdateProject`); the CLI edits one with `lampo project set`
(`PatchProjectEnvironment`, control request `project-patch`): a patch of extensions, add-ons,
php.ini values, services, database name, public folder and Apache directives, applied to the
stored project under that same lock, so a CLI edit never reverts a concurrent GUI save.
The same patch renames (`lampo project rename`, `--name`) and adds or removes addresses
(`--address`/`--no-address`, the displayed form parsed by `SiteAddress::parse`; `loopback`
draws a free port from the allocator in the same commit). Additions are appended (the first
address stays primary), at least one must remain, and `check_conflicts` refuses names and ports
of other sites. `lampo project remove` sends `project-delete` (`DeleteProject`, as the GUI);
`lampo project import` sends `project-import` (`ImportProject`, as the GUI).
From the CLI, a create, import or patch that adds or removes hosts-file names, or the removal
of a project that has some, fails with `ElevationRequired` *before* saving anything unless
`--allow-elevation`, so a retry with the flag finds the same state.

Custom Apache directives reach the vhost verbatim and come from the GUI, the CLI, agents and
imported files, so they are untrusted: `ApacheDirectives::new` refuses whatever runs programs,
loads code, changes Lampo's variables or PHP environment, or maps/grants access to folders
(`lampo_domain::apache_directives`). A stored value is only checked structurally when read, so
an old project still loads; `ProjectWriter` refuses to save it and the sync leaves its vhost
out (`config-rejected`, like a vhost `httpd -t` rejects) until it is fixed. An import never
brings the export's directives nor the php.ini keys that load or run files
(`lampo_domain::project_import`): they are returned in `ImportProjectResultDto` for the user
to review, and `tls` is reset.

Likewise the GUI saves the whole settings (`SettingsService::update`) while `lampo settings set`
sends a patch (`PatchSettings`, control request `settings-patch`): Apache's port, service
instance ports, the default PHP and the loopback range, applied to the stored settings. Both,
like the first PHP installed becoming the default and the first-run wizard's completion, save
through one `SettingsWriter`: under the allocator lock (so its check against the sites' loopback
ports cannot interleave with a project save), with the same checks. A port may not be a site's
loopback port (`Conflict`); new ports must be free (`PortInUse`) unless a Lampo service holds
them (a swap); a new default PHP must be installed (`PackageMissing`). Then the sync restarts
running services on their new port. The patch reply lists the changes, the sync report and
notes (stopped services, sites whose `.env` mentions the old port). TLD and close-to-tray stay
GUI-only; service instances cannot be added or removed by either front-end (a GUI save that
lists other instances than the stored ones is refused with `Validation`). Adopting the default
PHP is best effort: its failure is logged and never fails the install. `lampo settings get`
reads the file in-process.

Creating a project that uses MySQL also creates its database (name derived from the project id) on
its instance, starting it on demand; an existing database is kept, and a failure never fails
the creation: the result reports it in `databaseSetup` (`lampo project create --no-database`
or `createDatabase: false` skips it). Deleting a project never drops its database; its dedicated
MySQL account is dropped by the next provisioning of its instance (see
[MySQL credentials](#mysql-credentials)).

### Hosts file and the elevated helper

Lampo owns one marked block of the system hosts file (`<GetSystemDirectoryW()>\drivers\etc\hosts`,
on whatever drive Windows lives; the composition roots use `ElevatedHostsFile::system`, never a
hardcoded `C:\` path). Reading needs no rights; writing goes through `lampo-hosts-helper.exe`,
started with `ShellExecuteExW` verb `runas` (UAC prompt, owned by the app window when it is shown,
otherwise by the foreground window, i.e. the terminal for the CLI). The helper
(`lampo_infra_web::hosts::helper`, `hosts::request`) is the only code that runs elevated, so it
accepts as little as possible:

* **Fixed target**: release builds write only the system hosts file (`--hosts` exists in debug
  builds only, for tests).
* **Narrow content**: loopback addresses only, at most 4096 entries, and only names under the
  local TLDs (`lampo_domain::hostname::RESERVED_LOCAL_TLDS`: `.localhost`, `.test`, `.example`,
  `.invalid`, `.local`) or the TLD set in the settings (`--tld`, itself a `Tld`, which refuses
  `.com`, `.net`, `.org`, `.dev`, `.app`). `windowsupdate.com 127.0.0.1` is refused. The adapter
  applies the same check first, so a refused block never shows a prompt; a project address under
  another TLD makes the sync fail with an error naming it.
* **Untampered request**: the block is a JSON file in the user's `%TEMP%` (UAC passes neither
  stdin nor inherited handles, and the command line is too short for 4096 entries), removed when
  the helper exits. Its SHA-256 is on the command line (`--sha256`): the helper reads the file once,
  checks the digest and parses those same bytes, so a file swapped after the launch is refused.
* **Byte-level merge**: everything outside the block (BOM, CRLF/LF, the user's entries) is kept
  byte for byte; UTF-16 files and unterminated blocks are refused, not repaired.
* **Atomic, patient write**: temp file in `etc`, then `ReplaceFileW` (keeps ACL and attributes),
  retried up to 5 times with backoff (50 to 400 ms) while another process holds the file
  (sharing/lock violation).

**Residual risk.** The per-user installer puts the helper in `%LOCALAPPDATA%\Programs\Lampo`,
which the user, and any program running as the user, can write. Such a program could replace
`lampo-hosts-helper.exe` before Lampo starts it, and the user would then approve an elevated
binary that is not Lampo's: the checks above only bind the genuine helper. The UAC prompt (program name, path and, for an
unsigned binary, "Unknown publisher") is the remaining defence. Future mitigations: Authenticode-sign the helper
(the prompt then shows a verified publisher, and the app can verify the signature before
launching it) and/or install the helper per-machine under `%ProgramFiles%`, writable only by
administrators.

### New projects: git clone or Laravel

"Aggiungi progetto" (and `lampo project new`) can also *create* the folder before registering it:
`git clone --progress [--branch] -- <url> <dest>` (git from PATH, `GIT_TERMINAL_PROMPT=0`) or
`laravel new <name> --no-interaction ...` (starter kit, Pest/PHPUnit, `--git`, `--npm`/`--no-node`,
`--no-boost`). The Laravel Installer is managed by Lampo: `laravel/installer` lives in
`%LOCALAPPDATA%\Lampo\tools\laravel-installer\<php>` (required on first use, `composer update`d
best-effort afterwards) and runs with Lampo's PHP and Composer only: the child gets
`tools\composer\<php>\composer.bat` and the chosen PHP first on PATH and a tool php.ini
(`generated\php\_tools-<php>`, with zip/openssl/SQLite) as `PHPRC`; the global Composer is never
touched. It runs as a background task (`scaffold_project`) whose output lines stream as progress
messages; cancelling kills the whole process tree (one nested Job Object per run) and a folder
created by the run is removed on failure or cancel. Then detection and `create_project` follow
as for an existing folder. Rules (folder names, URLs, branches, command lines) are pure functions
in `lampo_domain::scaffold`. The GUI form remembers (in `localStorage`) the parent folder, starter
kit and testing framework of the last successful creation; the folder is offered again only while
`scaffold_parent_exists` confirms it (the addressing mode always follows `default_addressing`).

```powershell
lampo project new --git https://github.com/acme/shop.git --in D:\sites [--branch dev] [--folder shop]
lampo project new --laravel shop --in D:\sites [--php 8.4] [--starter-kit react|vue|livewire|svelte] [--phpunit] [--init-git] [--npm]
#   then registers it like `project create` ([--activate] [--no-database]; `--no-register` skips it)
```

### Claude Code integration

A "ghost" integration: nothing is ever written inside a project (no `AGENTS.md`, no project
`.claude/settings.json`, no project scope). Lampo generates a local plugin `lampo` in a local
marketplace `lampo` (id `lampo@lampo`, skills invoked as `lampo:<skill>`) into
`%LOCALAPPDATA%\Lampo\agent-plugins\claude-code\` and registers it **only through the official
CLI, in user scope**:

```text
claude plugin validate <dir>/plugins/lampo        claude plugin validate <dir>
claude plugin uninstall lampo@lampo --scope user  claude plugin marketplace remove lampo   (errors ignored)
claude plugin marketplace add <dir>                claude plugin install lampo@lampo --scope user
claude plugin list --json                          (verify: exact id match)
```

* **Generated files** (`lampo_domain::claude_code`, pure): `marketplace.json`, `plugin.json`
  (name, version, description, author, homepage/repository, MIT), a README, `hooks/hooks.json`,
  the icon (`.claude-plugin/icon.png`, from `claude-plugin/icon.png`, written as bytes) and the
  skills. The skills are every file under `claude-plugin/skills/<skill-id>/` in the
  repository, embedded at compile time by `crates/lampo-application/build.rs` (add files there,
  no Rust change needed). Files are UTF-8 without BOM, LF, relative forward-slash paths. Skill
  contents, the CLI drift test and the trigger evals: [CLAUDE-PLUGIN.md](CLAUDE-PLUGIN.md).
* **Version** `<app version>-h<fnv32>` hashed over every generated file (skills, hook, manifests,
  CLI path): Claude Code caches plugins per version, so it refreshes exactly when content
  changes. After a successful install the version is written to `.installed-version`.
* **`SessionStart` hook** (matcher `startup|resume|clear|compact`, timeout 10 s, exec form):
  `%LOCALAPPDATA%/Lampo/bin/lampo.exe agent context --hook`, the CLI copy in the shim dir that
  app updates refresh (so installing needs the command-line tools). It reads the hook's JSON on
  stdin (`cwd`, else the process cwd), resolves the site like `agent context` (file-based, never
  starts the app; live service states only from an app already running) and prints
  `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"…"}}` with a
  compact Markdown summary. Outside a Lampo project, or on any error, it prints nothing and
  exits 0.
* **When**: install and removal only on an explicit click (Settings > Integrazioni > Claude Code:
  Installa / Aggiorna / Rimuovi, with the step log); Lampo never touches `~/.claude` on its own
  the first time. `Startup` updates silently, concurrently with the environment sync, a plugin
  whose marker exists and differs from the expected version (file reads only to decide; a plugin
  removed with `claude plugin uninstall` is forgotten instead of reinstalled); a failure is logged
  and shown as a warning in the settings card.
* **`claude`** is found on the user's PATH (registry) or at `~\.local\bin\claude.exe`; npm shims
  run as `cmd /d /c claude …` with their folder first on PATH, all hidden, through the
  supervisor's `CommandRunner` (`ClaudeCliRunner`). `claude --version` (10 s) missing = "Claude
  Code non trovato". With `LAMPO_HOME` set nothing runs: the status is "isolated".
* **Uninstalling Lampo** (not an update, `/UPDATE`) unregisters the plugin when its marker exists
  (`NSIS_HOOK_POSTUNINSTALL`, errors ignored) and deletes the generated folder.

`cargo test -p lampo-infra-services --test claude_validate -- --ignored` runs the real
`claude plugin validate` on a generated plugin (it never installs anything).

## Process ownership and the CLI

The desktop app is the single owner of service processes (all in one Windows Job Object, each
service's process tree in a nested job of its own, so a forced stop leaves no worker behind). The
CLI runs read-only, file-based commands itself (status offline, project/worktree list,
`project detect`, `package extensions`, `settings get`, logs,
`env`, `exec`, `agent context`, `doctor`) and sends mutating ones to the app over a per-user
named pipe (`lampo_application::control`), starting the app headless (`--headless`, tray only)
when needed. Shims never talk to the app. `lampo open` (`OpenProjectIn`, the same use case as
the GUI's buttons) also runs in the CLI: the apps it starts (Explorer, VS Code, the browser, a
console) are the user's own processes, never supervised by Lampo.

The services are stopped gracefully however the app goes away, since the Job Object would
otherwise kill mysqld (InnoDB crash recovery, corrupted MyISAM `mysql.*` tables on 5.7): the
tray's "Esci" and an update run `Shutdown` themselves; at the end of the Windows session
(logoff, shutdown) a hidden top-level window registers a shutdown block reason on
`WM_QUERYENDSESSION` and stops the services on `WM_ENDSESSION`, bounded to a few seconds
(`apps/desktop/src-tauri/src/session_end.rs`, `orderly_exit.rs`); `RunEvent::Exit` runs the same
stop once (a no-op after a quit). Whatever is still running then, and on a panic of the main
thread, gets a last-resort `MYSQLShutdown<pid>` request with a short wait
(`lampo_infra_services::stop_mysql_servers`, synchronous, no app state needed). `Shutdown` first
closes the `ShutdownGate`, which `ApplyEnvironment` shares: from then on no sync (one already
running, the startup one, the one applying a postponed change) and no single-service call starts
or restarts a service (`app-unavailable`).

Inside the app, `ApplyEnvironment` is the single owner of the service lifecycle: full syncs and
single-service starts, restarts and stops (`lampo up/down/restart <service>`, database
operations) share its lock and its record of each service's launch, so a service started
outside a sync is still restarted when its configuration changes. A stop does not wait for a
start still waiting for readiness (a MySQL recovery can take up to an hour): it tells the
runner to give that start up before taking the lock (the start ends `cancelled`, a warning in a
sync), and the GUI keeps «Ferma» enabled while a service starts. A database operation (import,
export, worktree database clone) holds a lease on its MySQL instance: while it runs, a sync
postpones stopping or restarting that instance (with a warning) and a manual stop or restart is
a `conflict`; the postponed change is applied once the last lease is released. The lease names
the port the instance runs on, so other database operations meanwhile reach it there and not on
a new port the settings name. A sync that fails returns its first failure, with every failure
in `details.errors` (`[{code, message, details?}]`); the CLI adds the `--allow-elevation` hint
when one of them is `elevation-required`. The syncs that never prompt for elevation (worktree
environments, postponed changes) ignore a pending hosts change but return any other failure.

A running service that exits without being asked to (not a stop, not a start that fails) is a
crash: its runner publishes `ServiceCrashed`, and with the `auto_restart_services` setting on
(the default; `lampo settings set --auto-restart on|off`, the GUI's settings page)
`ServiceAutoRestart` starts it again through `ApplyEnvironment` after 2, 4 and 8 seconds, at most 3
attempts in a row, the count starting over once the service ran a minute
(`lampo_domain::auto_restart`). A service stopped since its crash (by the user or a sync: its
launch is no longer recorded) is left stopped, and nothing restarts while the app shuts down.
Every step is a `lampo://service-auto-restart` event, which the GUI shows as a notification.

`lampo` never prompts and never triggers UAC unless `--allow-elevation` is given.

### `lampo` exit codes (stable)

| Code | Meaning |
|-----:|---------|
| 0 | success |
| 1 | unclassified failure |
| 2 | invalid command line |
| 3 | not found |
| 4 | invalid input |
| 5 | conflict (duplicate, port in use, in use) |
| 6 | no project for the current directory |
| 7 | needs the hosts-file UAC prompt: re-run with `--allow-elevation` |
| 8 | UAC prompt refused |
| 9 | Lampo app not running and could not be started |
| 10 | required package not installed |
| 11 | network or checksum failure |
| 12 | external program failed |
| 13 | cancelled or timed out |
| 14 | `doctor` found problems |
| 15 | CLI/app version mismatch |

Shims return the tool's own exit code, or 127 when they cannot resolve a PHP/tool.

### MySQL client shims

`mysql` and `mysqldump` (`lampo_domain::shim::tools::{Mysql, Mysqldump}`) run the client of the
site's instance as `[leading option-file options] --host --port --user
--default-character-set=utf8mb4 [--database=<db>] <user args>`: option-file options the user
puts first (`--no-defaults`, `--defaults-file=`, `--login-path=`…) must stay first for the
client; every other user argument comes after the defaults, so it wins. `utf8mb4` is forced
because the clients otherwise pick the Windows console code page (`cp850`) even for piped
output, losing non-Latin-1 text, and 5.7's `mysqldump` would use 3-byte `utf8`.

The database is the site's (`ShimService.database`: a worktree environment's own database or
its parent's; `None` for the default target and sites without MySQL):

* `mysql` adds `--database=<db>` unless the arguments choose one: a positional argument, `-D`
  (separate, attached, grouped), `--database[=]` (`_`/`--loose-` forms too); `--database=` with
  an empty value is the opt-out (no database; no env variable is needed). The scanner
  (`shim/mysql_args.rs`) knows which short/long options take a separate value, so
  `-e "SHOW TABLES"` is not a database; `-p`/`--password`/`-#` take only attached values; `--`
  ends options. A misread errs towards "option value", which is harmless: the client lets a
  positional database override the earlier `--database`.
* `mysqldump` appends the database as the positional argument only when there is no positional
  and no `-A`/`--all-databases`/`-B`/`--databases`; existing positionals are never touched.
* Only a database that exists is added: without connecting, the shim looks for its folder in
  the instance's data dir (`ShimService.data_dir`, `shim/mysql_database.rs`; one `metadata`
  call, the name in MySQL's file-name encoding, `-` → `@002d`). A missing folder means the
  database has not been created yet: `mysql` connects without a default database and prints
  `lampo-shim: database '<db>' does not exist yet, connecting without a default database`
  (hidden by `LAMPO_QUIET`), so `mysql -e "CREATE DATABASE ..."` and dump imports work;
  `mysqldump` appends nothing and the client prints its own usage error. When the shim cannot
  tell (no data dir in an older index, a data dir that cannot be read, a non-ASCII or device
  name) it adds the database as before: a visible client error beats a silent change. No check
  runs when the user chose a database.

### MySQL credentials

Instances are initialised with `root` and no password (`--initialize-insecure`). Two kinds of
password are stored by `DpapiMysqlPasswords` (`lampo-infra-services`), one file each in
`secrets\`, encrypted with DPAPI for the current Windows user (another user, or a copy on another
machine, cannot decrypt them) under an entropy per kind, so a file of one kind never reads as the
other: root's (`<service-id>.root.dpapi`, ports `MysqlRootPasswordReader`/`Writer`) and each
project account's (`<project-id>.project.dpapi`, `MysqlProjectPasswordReader`/`Writer`; the first
save wins atomically, so two first uses agree on one password). New passwords come from
`SystemRandomPasswords` (`MysqlPasswordGenerator`: 32 letters and digits from `BCryptGenRandom`).

**Accounts.** A project's `mysql_credentials` (`lampo_domain::mysql::accounts::MysqlCredentials`)
is `project`, a dedicated account `lampo_<id>` (`MysqlUserName::for_project`: `-` as `_`, at most
32 characters, a long id ends in `_<fnv32 of the id>`), or `root`. Project files written before
the field read as `root`; new projects get `project` unless the setting
`mysql_project_credentials` (`lampo settings set --project-credentials on|off`, the GUI's settings
page, on by default; it needs no sync) is off; an import keeps what the export says (the `.env`
travels with the code). `lampo project set --mysql-credentials project|root` and the GUI's project
editor (`UpdateProjectRequest::mysql_credentials`, absent = unchanged) switch one project.

**Provisioning.** `InstanceAccounts::plan` (pure) gives, per instance, the dedicated account of
every project on it with exactly its database and the own databases of its worktree environments,
and `secure_root`: `true` when no project on the instance (active or not: an inactive project's
`.env` still names its account) uses `root`. `ApplyEnvironment` brings each running MySQL instance
to its plan (`MysqlAccounts::provision`) after every sync and after a start outside a sync
(`ServiceLifecycle::ensure`), remembering the plan applied per instance until it stops or starts
again, so an unchanged plan costs nothing; a failure is a sync warning and the next sync tries
again. When `secure_root` holds, root's password is taken from the store or generated and **stored
before it is applied**; `MysqlAccountAdmin::secure_root` (`MysqlCliAdmin`) probes it, and only if
the server refuses it (`ERROR 1045`) while accepting root without a password runs `ALTER USER` for
every `root` host. So the migration of an instance created without a root password happens at its
first start (or sync) once no project on it uses root, and a crash between saving and applying is
repaired by the next provisioning. A root password is never removed: a project switched back to
`root` (or created on root later) is shown that password. Then `sync_project_accounts` (as root)
drops the `lampo_%` accounts no project owns any more and, for each planned one and each of the
hosts `localhost` and `127.0.0.1` (TCP clients may match either), `CREATE USER IF NOT EXISTS` +
`ALTER USER … IDENTIFIED BY`, `REVOKE ALL PRIVILEGES, GRANT OPTION`, then `GRANT ALL PRIVILEGES ON
<db>.*` per database (`_`/`%` escaped: a grant names exactly that database, which the account may
then create). Statements carrying a password are written to a user-only temporary file
(`SecretTempFile`) that `mysql` reads on its standard input, never `--execute=`.

**Shown credentials.** `DatabaseConnection` names its account (`project_account`); the
application fills the password for display (`ConnectionPasswords`, implemented by
`MysqlPasswords`, which also generates a project's password the first time it is asked): the
project account's, or root's stored one (empty while root has none). `ProjectDto`,
`WorktreeEnvironmentDto` (the parent's account), `lampo project info` and `lampo agent context`
show them; `agent context` hints at the switch for a project still on root. Lampo never writes a
`.env`.

**Lampo's own clients.** Every client Lampo runs as root authenticates with the stored root
password when there is one: `MysqlCliAdmin` (statements, import, export, clone), the `mysqladmin
shutdown` fallback and the `mysql`/`mysqldump` shims. The shims of a project with a dedicated
account (and of its worktree environments) connect as that account instead (`ShimService::user`,
`project_account`) with its stored password; other sites and the default target use root.
Passwords never reach a command line or a log: `ClientOptionFile` writes a `[client]` option file
to `%TEMP%`, created with a descriptor granting access to the current user only, passed as the
client's first argument and deleted after the client exits. Lampo's own clients use
`--defaults-file=` (that file and no other, like the `--no-defaults` they use without a
password); the shims use `--defaults-extra-file=`, so the user's own option files keep applying,
and only when the user chose no account, password, server or option file
(`ShimInvocation::stored_credentials`: `StoredCredentials::Root(instance)` or
`Project(project)`): a stored password never reaches another account or server.

`lampo exec` and `lampo env` need nothing extra: the shim dir is first on their PATH and
`LAMPO_PROJECT`/`LAMPO_PROJECT_ROOT` are set, so `mysql` there is the shim with the same
defaults. `LAMPO_DB_USER` names the site's account; no variable carries a password: the
project variables also reach the vhost (`SetEnv`, a generated file in clear) and every child
process, so the password stays in the DPAPI store and the connection details. Lampo does not
export `MYSQL_HOST`/`MYSQL_TCP_PORT` (only `PHPRC` and `LAMPO_*`).

### Site resolution (shims and CLI)

`--project` (CLI only) → `LAMPO_PROJECT` (`shop` or `shop@feature-x`) → longest-prefix match of
the cwd against project and worktree-environment folders → git worktree of a registered project
(reads `.git` files, never spawns git) → the same two folder matches once more with the cwd's
final path (`GetFinalPathNameByHandleW`: `subst` drives, junctions, 8.3 names; only when the
textual match failed) → global default PHP. Verbatim paths (`\\?\C:\x`, `\\?\UNC\…`) are read in
their plain form.

`lampo exec`, `lampo env` and the app's "Apri terminale" export `LAMPO_PROJECT` together with
`LAMPO_PROJECT_ROOT`, the folder it was resolved for. The variable outlives that folder (in the
terminal of `shop`, `cd ..\blog`), so it holds only while the cwd is inside `LAMPO_PROJECT_ROOT`
(or the site's registered folder when the root is not set) or outside every project; in the
folder of another project that project wins, and the shim or `lampo` prints one line on stderr
(`LAMPO_PROJECT=shop ignored: this folder belongs to blog`), hidden by `LAMPO_QUIET=1`.

`lampo which <tool>` (`ShimToolQuery`) runs this chain and the tool's `ShimTool::invocation`
without starting anything, and reports the step that answered: `ResolutionSource::Explicit`
is split back into `--project` and `LAMPO_PROJECT` (the flag is tried first, so an explicit
answer comes from it whenever it was given).

The shims read only `%LOCALAPPDATA%` (from the environment; the known folder is the fallback)
and never compute the config root. The tool a shim or `lampo exec` starts is tied to it by a
Job Object (`lampo_cli::job`): the launcher joins the job before the spawn, so if an IDE, Node or
CI kills the launcher, the tool and everything it started (`php artisan serve` and its PHP
server) die with it; when the tool exits on its own the job is disarmed first, so programs it
detached keep running. Closing the console (the window or a Windows Terminal tab), logging off
or shutting down can end the launcher with `ExitProcess` while the tool still runs, which skips
that step: a console handler disarms the job on `CTRL_CLOSE_EVENT`, `CTRL_LOGOFF_EVENT` and
`CTRL_SHUTDOWN_EVENT`, so a `code .` or `start notepad` from "Apri terminale" survives the window
(the tool attached to the console still gets the event and closes). Only a forced kill leaves
the job armed.

## Building

```powershell
cd apps/desktop; pnpm install; pnpm build; cd ../..   # build/ must exist for the Tauri crate
cargo check --workspace
cargo test -p lampo-domain
cd apps/desktop; pnpm check; pnpm tauri dev
```

`.cargo/config.toml` links the C runtime statically (`+crt-static`) into every Windows binary:
`lampo.exe`, the shims and the app start on a machine without the Visual C++ redistributable
(check with `objdump -p <exe>` or `dumpbin /dependents <exe>`: no `VCRUNTIME140.dll`). A
`RUSTFLAGS` variable replaces those flags, so CI must not set one.

Data lives in `%APPDATA%\Lampo` (config, one TOML per project) and `%LOCALAPPDATA%\Lampo`
(packages, MySQL data, logs, generated configs, shims). `LAMPO_HOME` overrides both (tests).
Logs stay below 10 MiB per file (`lampo_infra_fs::log_rotation`): Apache's error and access
logs are piped through `<SRVROOT>\bin\rotatelogs.exe -n 3` (`error.log`, `error.log.1`,
`error.log.2`, round again), a service's `output.log` is moved to `output.log.1` as it is
written, MySQL's error log at each start, and PHP's `php_errors.log` files when their php.ini
is written and every hour while the app runs (`PeriodicLogRotation`). The log reader shows the
most recently written file of each set.
The app itself is installed elsewhere, in `%LOCALAPPDATA%\Programs\Lampo` (per user, no admin):
Tauri's per-user NSIS default is `%LOCALAPPDATA%\<productName>`, which is the data root, so
`apps/desktop/src-tauri/windows/installer-hooks.nsh` redirects exactly that folder and the
directory page refuses it. Updates reuse the location the installer recorded in the registry
(`HKCU\Software\lampo\Lampo`); the uninstaller deletes only the files it installed, and its
"delete app data" option only `%APPDATA%\io.lampo.desktop` and `%LOCALAPPDATA%\io.lampo.desktop`
(WebView2 data), never the Lampo data roots. An
instance with `LAMPO_HOME` set is isolated: it uses its own control pipe (so its CLI never talks
to the real app) and never edits the user PATH (put `%LAMPO_HOME%\data\bin` on PATH in your own
shell instead).

The helper executables (`lampo.exe`, `lampo-shim.exe`, `lampo-hosts-helper.exe`) are looked up
next to `lampo-desktop.exe`: in development they are all in `target\debug`; a bundle ships them as
Tauri external binaries (`pnpm bundle` = `tauri build --config src-tauri/tauri.bundle.conf.json`,
staged by `scripts\prepare-sidecars.ps1`).

Installing the shims also copies `lampo.exe` into the shim dir, so its one PATH entry provides
the CLI too; `Startup` refreshes that copy with the shims after an update
(`ShimInstaller::refresh`, byte comparison). The copy does not sit next to the app, so at every
start the app records its own folder in `<data>\app-location.txt` (`AppLocationRecorder`), and the
CLI reads it whenever `lampo-desktop.exe` is not next to itself
(`lampo_infra_store::app_location::resolve_app_dir`) to find the app and the helpers. The last
started app wins: a dev build started on the real data root makes the copy a `target\debug` CLI that starts the dev app,
until the installed app runs again. Under `LAMPO_HOME` the copy and the file
are both in the isolated data root.

Releases, signing and in-app updates: [RELEASE.md](RELEASE.md), [UPDATER.md](UPDATER.md).

### End-to-end check

`scripts\e2e-cli.ps1` drives the real CLI, app, Apache, PHP and MySQL with real downloads in an
isolated `LAMPO_HOME` (no hosts file, PATH or registry changes, no UAC): headless start, package
installs, two projects on different PHP versions, HTTP + `.htaccess`, shims, databases, a git
worktree environment with a cloned database, `agent context`, logs, and process cleanup after
`lampo down`, `lampo quit` and a hard kill of the app.

```powershell
cargo build --workspace --bins
powershell -ExecutionPolicy Bypass -File scripts\e2e-cli.ps1 [-DataRoot D:\lampo-e2e]
```
