# Lampo: architecture

A lightweight local PHP development environment for Windows: one Apache (mod_fcgid) with a
virtual host per project, any PHP from 5.6 up, shared MySQL instances per version, optional Redis,
per-git-worktree environments for parallel coding agents. Rust core + Tauri 2 + SvelteKit GUI,
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
| `crates/lampo-infra-web` | Apache config templates + `httpd -t` + runner, php.ini rendering, hosts-file block via the elevated `lampo-hosts-helper` | Apache/PHP/hosts specifics |
| `crates/lampo-infra-services` | Job Object supervisor, MySQL/Redis runners, port probe, log files, `DatabaseAdmin`, control named pipe, headless app launcher, `claude` CLI runner | Local processes and IPC |
| `crates/lampo-cli` | `lampo` CLI and `lampo-shim` (copied as `php.exe` etc.): two composition roots (`src/composition.rs`, `src/bin/lampo-shim.rs`, which registers the shim tools) | CLI surface |
| `claude-plugin` | Claude Code plugin sources: `skills/<skill-id>/SKILL.md` + reference files and `icon.png` (embedded as is), trigger evals and a dev manifest (repository only) | What agents are taught |
| `apps/desktop` | SvelteKit GUI (`src/lib/api` typed API, `src/lib/messages` Italian strings) and `src-tauri` (Tauri commands, tray, composition root) | GUI |

Key rules: the domain does no IO and depends on no workspace crate; use cases see only ports;
traits are small and role-specific; new package kinds, presets, project detectors, resolvers,
shim tools, doctor checks and service kinds are added by implementing a trait and registering
it in the composition roots.

A **service kind** (MySQL, Redis, a future PostgreSQL or Mailpit) is data: a
`ServiceDescriptor` (default port, data dir, error log, client defaults, versioned or not;
built-ins in `lampo_domain::service::builtin`) plus a `ServiceRunner`. Projects list the
services they use (`environment.services`), settings list the configured instances
(`services`); Apache stays special as the web tier. Files written before this (`mysql`/`redis`
fields, `mysql_instances`/`redis_port`, shim index v1) are migrated on read.

Loopback ports are handed out by one `LoopbackPortAllocator` (shared by project creation and
`worktree up`), which also serialises every project save with its conflict check.

The GUI saves a whole project (`UpdateProject`); the CLI edits one with `lampo project set`
(`PatchProjectEnvironment`, control request `project-patch`): a patch of extensions, add-ons,
php.ini values, services, database name, public folder and Apache directives, applied to the
stored project under that same lock, so a CLI edit never reverts a concurrent GUI save. It
also checks what the GUI only offers as choices: add-ons installed for the project's PHP,
bundled extensions present in its `ext` dir, MySQL series configured in the settings.

Creating a project that uses MySQL also creates its database (name derived from the project id) on
its instance, starting it on demand; an existing database is kept, and a failure never fails
the creation: the result reports it in `databaseSetup` (`lampo project create --no-database`
or `createDatabase: false` skips it). Deleting a project never drops its database.

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
in `lampo_domain::scaffold`.

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

The desktop app is the single owner of service processes (all in one Windows Job Object). The
CLI runs read-only, file-based commands itself (status offline, project/worktree list,
`project detect`, `package extensions`, logs,
`env`, `exec`, `agent context`, `doctor`) and sends mutating ones to the app over a per-user
named pipe (`lampo_application::control`), starting the app headless (`--headless`, tray only)
when needed. Shims never talk to the app.

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

### Site resolution (shims and CLI)

`LAMPO_PROJECT` (`shop` or `shop@feature-x`) → longest-prefix match of the cwd against project
and worktree-environment folders → git worktree of a registered project (reads `.git` files,
never spawns git) → global default PHP.

## Building

```powershell
cd apps/desktop; pnpm install; pnpm build; cd ../..   # build/ must exist for the Tauri crate
cargo check --workspace
cargo test -p lampo-domain
cd apps/desktop; pnpm check; pnpm tauri dev
```

Data lives in `%APPDATA%\Lampo` (config, one TOML per project) and `%LOCALAPPDATA%\Lampo`
(packages, MySQL data, logs, generated configs, shims). `LAMPO_HOME` overrides both (tests).
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
