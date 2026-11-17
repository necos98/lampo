// GENERATED from the Rust DTOs by `cargo test -p lampo-application --lib ts_bindings -- --ignored`
// (crates/lampo-application/src/ts_bindings.rs). Do not edit: change the DTOs and regenerate.

// --- Errors ---

/**
 * Stable error categories. Adding a variant is a contract change: regenerate the TS bindings
 * (`ts_bindings`), update the GUI's code list (`src/lib/api/errors.ts`), the Italian messages
 * and the CLI exit-code table.
 */
export type ErrorCode = "validation" | "not-found" | "conflict" | "project-not-resolved" | "elevation-required" | "elevation-denied" | "app-unavailable" | "package-missing" | "network" | "checksum-mismatch" | "config-rejected" | "port-in-use" | "process-failed" | "io" | "cancelled" | "timeout" | "unsupported" | "protocol-mismatch" | "internal";

export type AppError = { code: ErrorCode, 
/**
 * English, developer-facing: front-ends show their own text for `code`.
 */
message: string, 
/**
 * Structured extras the GUI puts into its own text: `{ output }` (a string, the program's
 * output) for `config-rejected` and a failed `process-failed` step, plus `excludedSites` when
 * some sites were left out; `{ port, owner }` for `port-in-use`; `{ entity, id }` for
 * `not-found`; `{ package, php? }`, `{ tool }` or `{ redistributable }` for
 * `package-missing`; `{ package, usedBy }` for a package still in use (`conflict`);
 * `{ field: "envVars", reason, name?, part?, reserved? }` for a refused environment variable
 * (`validation`; never its value).
 */
details?: unknown, };

// --- Projects ---

/**
 * How a site is reached. Wire form: `{ "kind": "hostname", "host": "shop.test" }`,
 * `{ "kind": "localhost", "host": "shop.localhost" }`, `{ "kind": "loopback-port", "port": 8101 }`.
 */
export type SiteAddress = { "kind": "hostname", host: string, } | { "kind": "localhost", host: string, } | { "kind": "loopback-port", port: number, };

/**
 * Default addresses for a new project when none are given explicitly.
 */
export type Addressing = "standard" | "no-elevation" | "hosts-only";

export type TlsMode = "disabled" | "local-ca";

/**
 * The MySQL account a project's code connects as.
 */
export type MysqlCredentials = "root" | "project";

/**
 * The environment a preset carries and a project copies.
 */
export type ProjectEnvironment = { 
/**
 * PHP minor, e.g. `"8.3"`.
 */
phpVersion: string, 
/**
 * Bundled extensions (`"intl"`, `"pdo_mysql"`).
 */
extensions: Array<string>, 
/**
 * Add-on extension package kinds (`"xdebug"`, `"php-redis"`).
 */
addons: Array<string>, phpIni: { [key in string]: string }, 
/**
 * Relative to the project folder; `""` = the folder itself.
 */
publicDir: string, 
/**
 * Services the project needs while served (`{ "kind": "mysql", "version": "8.4" }`,
 * `{ "kind": "redis", "version": null }`). The MySQL entry also holds its database.
 */
services: Array<ServiceUse>, apacheDirectives: string, 
/**
 * Variables set for the site's PHP on the web (`SetEnv`) and on the command line (`lampo
 * env`, `lampo exec`, the `php`/`composer` shims). Names managed by Lampo (`PATH`, `PHPRC`,
 * `LAMPO_*`, …) are refused. Omitted = none.
 */
envVars: { [key in string]: string }, };

/**
 * One service a project uses: its kind and, for versioned kinds, the series.
 */
export type ServiceUse = { kind: string, version: string | null, };

/**
 * Addresses to offer for a project id, built by the domain rules and the port allocator
 * (`project suggest-addresses`): one per kind, plus the two default sets.
 */
export type AddressSuggestions = { 
/**
 * `<id>.<tld>` (hosts file, needs the UAC prompt once).
 */
hostname: SiteAddress, 
/**
 * `<id>.localhost`.
 */
localhost: SiteAddress, 
/**
 * The loopback port the allocator would assign now (not reserved).
 */
loopbackPort: SiteAddress, 
/**
 * `AddressingDto::NoElevation` defaults: localhost + loopback port.
 */
noElevation: Array<SiteAddress>, 
/**
 * `AddressingDto::Standard` defaults: hostname + localhost.
 */
standard: Array<SiteAddress>, 
/**
 * `AddressingDto::HostsOnly` defaults: the hostname alone.
 */
hostsOnly: Array<SiteAddress>, 
/**
 * The global default (`SettingsDto::default_addressing`), to preselect.
 */
defaultAddressing: Addressing, };

/**
 * Connection details shown to the user/agent. Lampo never writes them to `.env`.
 */
export type DatabaseConnection = { mysqlVersion: string, host: string, port: number, user: string, password: string, database: string, };

/**
 * Where a site's mail goes (the shared Mailpit), shown like the database connection.
 */
export type MailConnection = { host: string, 
/**
 * Plain SMTP: no authentication needed (any credentials are accepted), no encryption.
 */
smtpPort: number, 
/**
 * The web UI showing the caught messages (`http://127.0.0.1:8025`).
 */
webUrl: string, };

/**
 * Why a project warning is raised, for a front-end that words it itself.
 */
export type ProjectWarningCode = "legacy-php-mysql-auth";

/**
 * A warning about a project's configuration.
 */
export type ProjectWarning = { code: ProjectWarningCode, 
/**
 * English text, for the CLI and a code the front-end does not know.
 */
message: string, };

export type Project = { id: string, name: string, root: string, 
/**
 * `false` when the folder was moved/deleted (offer "relink").
 */
folderExists: boolean, addresses: Array<SiteAddress>, 
/**
 * Primary URL.
 */
url: string, 
/**
 * One URL per address, primary first.
 */
urls: Array<string>, documentRoot: string, environment: ProjectEnvironment, databaseName: string | null, 
/**
 * Connection details to show (never written to `.env`).
 */
database: DatabaseConnection | null, 
/**
 * Which account `database` names; meaningful only with MySQL. Absent (older apps) = root.
 */
mysqlCredentials: MysqlCredentials, 
/**
 * SMTP endpoint and web UI of Mailpit, when the project uses it.
 */
mail?: MailConnection | null, tls: TlsMode, active: boolean, originPreset: string | null, 
/**
 * Worktree environments of this project.
 */
worktrees: Array<WorktreeEnvironment>, 
/**
 * What a save has to tell about the saved project (`bcmath is built into PHP 8.3, nothing
 * to enable`); always empty outside a save's answer.
 */
notices: Array<string>, 
/**
 * What may keep the project from working as configured (PHP before 7.4 on MySQL 8.4),
 * whenever the project is shown. Absent (older apps) = none.
 */
warnings: Array<ProjectWarning>, };

/**
 * A project file that does not load (malformed, or edited into invalid values). The project is
 * hidden until the file is repaired; its loopback ports stay reserved.
 */
export type UnreadableProject = { file: string, reason: string, };

/**
 * Every registered project: the ones that load and the files that do not.
 */
export type ProjectList = { projects: Array<Project>, unreadable: Array<UnreadableProject>, };

/**
 * Compact row for `lampo status` / `lampo project list`.
 */
export type ProjectSummary = { id: string, name: string, root: string, active: boolean, url: string, phpVersion: string, mysqlVersion: string | null, worktreeCount: number, };

/**
 * Create a project. Only `root` is required: anything omitted is filled from the preset
 * (`preset_id`, else the detected one, else `generic-php`), detection and settings.
 */
export type CreateProjectRequest = { root: string, 
/**
 * Default: folder name.
 */
name?: string | null, 
/**
 * Default: derived from the name.
 */
id?: string | null, presetId?: string | null, 
/**
 * Overrides the preset's PHP minor.
 */
phpVersion?: string | null, 
/**
 * Full environment; when present the preset is only recorded as origin.
 */
environment?: ProjectEnvironment | null, 
/**
 * Explicit addresses; when absent `addressing` decides.
 */
addresses?: Array<SiteAddress> | null, addressing?: Addressing, activate?: boolean, 
/**
 * Create the project's MySQL database (if it uses MySQL), starting the instance on demand.
 * Default `true` (also when the field is omitted). A failure never fails the creation: it
 * is reported in [`CreateProjectResultDto::database_setup`].
 */
createDatabase?: boolean, 
/**
 * Environment variables added to the environment (the preset's or `environment`), each
 * replacing one with the same name (`lampo project create --env KEY=VALUE`).
 */
envVars?: { [key in string]: string }, };

/**
 * What happened to a new project's database (`CreateProjectResultDto::database_setup`).
 */
export type DatabaseSetupStatus = "created" | "already-exists" | "package-missing" | "failed";

export type DatabaseSetup = { 
/**
 * The project's database name.
 */
name: string, status: DatabaseSetupStatus, 
/**
 * Why, for `package-missing` and `failed`.
 */
message: string | null, };

/**
 * Result of creating a project: the project itself (flattened, so on the wire this *is* a
 * [`ProjectDto`] plus one optional field) and what happened to its database.
 */
export type CreateProjectResult = { 
/**
 * `null`/absent when nothing was attempted: the project uses no MySQL, or
 * `createDatabase` was `false`.
 */
databaseSetup?: DatabaseSetup | null, id: string, name: string, root: string, 
/**
 * `false` when the folder was moved/deleted (offer "relink").
 */
folderExists: boolean, addresses: Array<SiteAddress>, 
/**
 * Primary URL.
 */
url: string, 
/**
 * One URL per address, primary first.
 */
urls: Array<string>, documentRoot: string, environment: ProjectEnvironment, databaseName: string | null, 
/**
 * Connection details to show (never written to `.env`).
 */
database: DatabaseConnection | null, 
/**
 * Which account `database` names; meaningful only with MySQL. Absent (older apps) = root.
 */
mysqlCredentials: MysqlCredentials, 
/**
 * SMTP endpoint and web UI of Mailpit, when the project uses it.
 */
mail?: MailConnection | null, tls: TlsMode, active: boolean, originPreset: string | null, 
/**
 * Worktree environments of this project.
 */
worktrees: Array<WorktreeEnvironment>, 
/**
 * What a save has to tell about the saved project (`bcmath is built into PHP 8.3, nothing
 * to enable`); always empty outside a save's answer.
 */
notices: Array<string>, 
/**
 * What may keep the project from working as configured (PHP before 7.4 on MySQL 8.4),
 * whenever the project is shown. Absent (older apps) = none.
 */
warnings: Array<ProjectWarning>, };

/**
 * Result of removing a project: what was kept on purpose and is no longer reachable from Lampo.
 */
export type DeleteProjectResult = { 
/**
 * The own databases of its worktree environments, left in MySQL (a removal never drops a
 * database): drop them by hand if they are not needed.
 */
orphanedDatabases: Array<string>, };

export type UpdateProjectRequest = { projectId: string, name: string, addresses: Array<SiteAddress>, environment: ProjectEnvironment, databaseName: string | null, tls: TlsMode, 
/**
 * The MySQL account its code connects as. Absent (older clients) = unchanged.
 */
mysqlCredentials?: MysqlCredentials | null, };

export type SetProjectActiveRequest = { projectId: string, active: boolean, };

export type SetProjectPhpRequest = { projectId: string, phpVersion: string, };

export type RelinkProjectRequest = { projectId: string, root: string, };

export type ExportProjectRequest = { projectId: string, 
/**
 * Destination file (`.toml`).
 */
file: string, };

export type ImportProjectRequest = { file: string, 
/**
 * Folder on this machine, if different from the exported one.
 */
root: string | null, };

/**
 * The imported project (the fields of [`ProjectDto`]) and the settings of the file that were
 * left out because they could run code here. Front-ends show them so the user can add them
 * back knowingly.
 */
export type ImportProjectResult = { 
/**
 * The export's custom Apache directives, verbatim (never imported); `null` when it had none.
 */
droppedApacheDirectives: string | null, 
/**
 * php.ini keys left out (`auto_prepend_file`, `extension`, …), as written in the file.
 */
droppedPhpIniKeys: Array<string>, 
/**
 * Names of the environment variables left out (never imported: a variable such as
 * `COMSPEC` or `NODE_OPTIONS` can run code here). The values are not repeated.
 */
droppedEnvVars: Array<string>, id: string, name: string, root: string, 
/**
 * `false` when the folder was moved/deleted (offer "relink").
 */
folderExists: boolean, addresses: Array<SiteAddress>, 
/**
 * Primary URL.
 */
url: string, 
/**
 * One URL per address, primary first.
 */
urls: Array<string>, documentRoot: string, environment: ProjectEnvironment, databaseName: string | null, 
/**
 * Connection details to show (never written to `.env`).
 */
database: DatabaseConnection | null, 
/**
 * Which account `database` names; meaningful only with MySQL. Absent (older apps) = root.
 */
mysqlCredentials: MysqlCredentials, 
/**
 * SMTP endpoint and web UI of Mailpit, when the project uses it.
 */
mail?: MailConnection | null, tls: TlsMode, active: boolean, originPreset: string | null, 
/**
 * Worktree environments of this project.
 */
worktrees: Array<WorktreeEnvironment>, 
/**
 * What a save has to tell about the saved project (`bcmath is built into PHP 8.3, nothing
 * to enable`); always empty outside a save's answer.
 */
notices: Array<string>, 
/**
 * What may keep the project from working as configured (PHP before 7.4 on MySQL 8.4),
 * whenever the project is shown. Absent (older apps) = none.
 */
warnings: Array<ProjectWarning>, };

/**
 * Result of analysing a folder before creating a project.
 */
export type Detection = { folder: string, 
/**
 * `laravel`, `wordpress`, `symfony`, `composer` or `null`.
 */
projectType: string | null, presetId: string | null, phpVersion: string | null, suggestedName: string, suggestedId: string, evidence: Array<string>, 
/**
 * Service kinds the folder's own files show it uses (`mysql`, `redis`; from a Docker
 * Compose file). Creation adds the unversioned ones the preset lacks.
 */
services: Array<string>, 
/**
 * What Lampo cannot provide for this folder (PostgreSQL, search engines…), developer-facing.
 */
warnings: Array<string>, 
/**
 * Id of the project already registered for this folder, if any.
 */
alreadyRegistered: string | null, 
/**
 * One proposed project per PHP app of a Docker Compose file that runs several, each from
 * its own folder; omitted otherwise (the fields above describe the folder itself).
 */
apps?: Array<DetectedApp>, };

/**
 * One PHP app of a Docker Compose file: what `project create` would need for it.
 */
export type DetectedApp = { 
/**
 * The compose service running it (`shop`).
 */
service: string, 
/**
 * Its code, relative to the analysed folder (`apps/shop`), or the host path as written
 * when it lies outside.
 */
folder: string, phpVersion: string | null, 
/**
 * Extensions its Dockerfile installs, sorted.
 */
extensions: Array<string>, 
/**
 * Relative to `folder` (`public`, `.`); `null` when not found.
 */
documentRoot: string | null, 
/**
 * The service's `environment:` variables, as written in the compose file.
 */
environment: { [key in string]: string }, 
/**
 * Services it reaches by host name (`depends_on`, `DB_HOST=mysql`), sorted.
 */
serviceHosts: Array<string>, };

// --- New project setup ---

export type StarterKit = "none" | "react" | "vue" | "livewire" | "svelte";

export type TestingFramework = "pest" | "phpunit";

/**
 * Wire form: `{ "kind": "git-clone", "url": "...", "branch": null }`,
 * `{ "kind": "laravel", "phpVersion": "8.4", "starterKit": "react", "testing": "pest", "git": true, "npm": false }`.
 */
export type ScaffoldSource = { "kind": "git-clone", url: string, 
/**
 * Branch or tag to check out (default: the remote's default branch).
 */
branch?: string | null, } | { "kind": "laravel", 
/**
 * PHP minor running the installer (`"8.4"`); default: the newest installed PHP.
 */
phpVersion?: string | null, starterKit?: StarterKit, testing?: TestingFramework, 
/**
 * Initialise a git repository (`--git`, needs git).
 */
git?: boolean, 
/**
 * Install and build the front-end (`--npm`, needs Node.js).
 */
npm?: boolean, };

/**
 * Creates `<parent_dir>\<folder_name>` from a source. Runs as a background task; on success the
 * front-end continues with detection and `CreateProjectRequest` on [`ScaffoldProjectResultDto::folder`].
 */
export type ScaffoldProjectRequest = { 
/**
 * Existing folder the new project folder is created in.
 */
parentDir: string, 
/**
 * Name of the new folder. Required for Laravel; for a clone it defaults to the repository
 * name (`https://github.com/acme/shop.git` → `shop`).
 */
folderName?: string | null, source: ScaffoldSource, };

export type ScaffoldProjectResult = { 
/**
 * The folder that was created (absolute Windows path).
 */
folder: string, 
/**
 * PHP minor the Laravel Installer ran with (`None` for a clone): a good default for the project.
 */
phpVersion: string | null, };

// --- Presets ---

export type Preset = { id: string, 
/**
 * English fallback; the GUI localises built-ins by `id`.
 */
name: string, description: string, builtin: boolean, environment: ProjectEnvironment, };

export type SavePresetRequest = { 
/**
 * Project whose environment is copied.
 */
projectId: string, name: string, description?: string, };

// --- Packages and tasks ---

export type PackageId = { 
/**
 * `php`, `apache`, `mod-fcgid`, `mysql`, `redis`, `composer`, `xdebug`, `php-redis`.
 */
kind: string, version: string, 
/**
 * For add-on extensions: the PHP minor they are built for.
 */
phpVersion: string | null, };

export type CatalogChannel = "frozen" | "live";

export type CatalogEntry = { id: PackageId, channel: CatalogChannel, endOfLife: boolean, sizeBytes: number | null, 
/**
 * `vc11`, `vc14`, `vc15`, `vs16`, `vs17`.
 */
vcRuntime: string | null, 
/**
 * Whether the required VC++ runtime is installed (`null` if none required).
 */
vcRuntimeInstalled: boolean | null, installed: boolean, 
/**
 * `<major>.<minor>` of the version (`8.3`), as the domain computes it.
 */
series: string, 
/**
 * Tab the GUI lists it under: `php`, a service kind (`mysql`, `redis`, `apache`),
 * `addons` (PHP-pinned extensions) or `tools` (anything else).
 */
group: string, };

export type InstalledPackage = { id: PackageId, path: string, 
/**
 * Site ids (`shop`, `shop@fix-1`) or `system` using this package.
 */
usedBy: Array<string>, 
/**
 * `<major>.<minor>` of the version.
 */
series: string, };

/**
 * Install a package. Without `version`: newest in `series` (if given) else newest overall.
 */
export type InstallPackageRequest = { kind: string, version?: string | null, 
/**
 * `"8.3"` → latest 8.3.x.
 */
series?: string | null, phpVersion?: string | null, };

export type UpdateAvailable = { kind: string, series: string, phpVersion: string | null, installed: string, latest: string, };

/**
 * Returned by commands that start a background task; progress arrives as events.
 */
export type TaskStarted = { taskId: string, };

/**
 * An extension an installed PHP minor can load (from its `ext` folder).
 */
export type PhpExtension = { 
/**
 * As written in a project's extension list (`intl`, `pdo_mysql`).
 */
name: string, 
/**
 * Loaded with `zend_extension` (`opcache`).
 */
zend: boolean, };

/**
 * What a background task does, as requested by the front-end that started it. Lets a window
 * created later (the GUI destroys its window on close) find and follow tasks still running.
 * Wire form: `{ "kind": "install-package", "request": {...} }`, `{ "kind": "first-run" }`,
 * `{ "kind": "worktree-up", "request": {...} }`, `{ "kind": "scaffold-project", "request": {...} }`,
 * `{ "kind": "uninstall-package", "id": {...} }`.
 */
export type TaskOperation = { "kind": "install-package", request: InstallPackageRequest, } | { "kind": "first-run" } | { "kind": "worktree-up", request: WorktreeUpRequest, } | { "kind": "scaffold-project", request: ScaffoldProjectRequest, } | { "kind": "uninstall-package", id: PackageId, };

/**
 * A background task that has not finished yet.
 */
export type RunningTask = { taskId: string, operation: TaskOperation, 
/**
 * Latest progress report, if any arrived yet.
 */
progress: ProgressReport | null, 
/**
 * Latest progress report per subject (a first run installs several packages).
 */
subjects: Array<ProgressReport>, };

export type FirstRunItem = { kind: string, 
/**
 * Series to install (`"8.4"`), or `null` for latest.
 */
series: string | null, installed: boolean, };

export type FirstRunStatus = { completed: boolean, 
/**
 * Apache, mod_fcgid, latest PHP, MySQL default series, Composer.
 */
items: Array<FirstRunItem>, };

// --- Services ---

export type ServiceState = "stopped" | "starting" | "running" | "stopping" | "failed" | "unknown";

export type ServiceStatus = { 
/**
 * `apache`, `redis`, `mysql-8.4`.
 */
service: string, 
/**
 * `apache`, `mysql`, `redis`.
 */
kind: string, 
/**
 * Display label from the kind's descriptor (`MySQL 8.4`).
 */
label: string, state: ServiceState, pid: number | null, port: number | null, 
/**
 * Web UI port of kinds that have one (Mailpit), configured or default.
 */
uiPort?: number | null, 
/**
 * `http://127.0.0.1:<uiPort>`: what "open the web UI" opens.
 */
uiUrl?: string | null, message: string | null, 
/**
 * Whether the current plan needs it running.
 */
required: boolean, 
/**
 * Site ids needing it; empty + `required` = needed by the system (always-on MySQL).
 */
requiredBy: Array<string>, };

/**
 * Selects services; empty = every service in the current plan.
 */
export type ServicesRequest = { services: Array<string>, };

/**
 * What an environment sync / up / down did.
 */
export type SyncReport = { started: Array<string>, stopped: Array<string>, restarted: Array<string>, hostsFileUpdated: boolean, warnings: Array<string>, };

/**
 * `lampo status`.
 */
export type Status = { appRunning: boolean, services: Array<ServiceStatus>, projects: Array<ProjectSummary>, worktreeCount: number, 
/**
 * Project files that do not load (absent from `projects`). Absent (older apps) = none.
 */
unreadableProjects: Array<UnreadableProject>, };

// --- Worktree environments ---

/**
 * Database mode chosen when a worktree environment is created.
 * Wire form: `{ "mode": "shared" }`, `{ "mode": "fresh" }`, `{ "mode": "clone", "source": "shop" }`.
 */
export type DatabaseMode = { "mode": "shared" } | { "mode": "fresh" } | { "mode": "clone", source: string, };

export type WorktreeEnvironment = { 
/**
 * `shop@feature-login`.
 */
site: string, projectId: string, slug: string, 
/**
 * Worktree folder.
 */
root: string, branch: string | null, 
/**
 * Canonical URL (`http://127.0.0.1:8101`).
 */
url: string, 
/**
 * Canonical URL first, then `http://<slug>.<project>.localhost[:port]`.
 */
urls: Array<string>, loopbackPort: number, localhostAlias: string, documentRoot: string, databaseMode: DatabaseMode, database: DatabaseConnection | null, 
/**
 * The folder no longer exists (prunable).
 */
stale: boolean, createdAtUnixMs: number, };

/**
 * `lampo worktree up [path]`: idempotent.
 */
export type WorktreeUpRequest = { 
/**
 * Any folder inside the worktree.
 */
path: string, 
/**
 * Parent project; default: resolved from the worktree's main repository.
 */
projectId?: string | null, 
/**
 * Ignored when the environment already exists.
 */
database?: DatabaseMode, };

export type WorktreeUpResult = { environment: WorktreeEnvironment, 
/**
 * `false` when it already existed.
 */
created: boolean, };

export type WorktreeDownRequest = { 
/**
 * Worktree folder or site id (`shop@feature-login`).
 */
target: string, 
/**
 * Also drop the environment's own database (never the parent's).
 */
dropDatabase?: boolean, };

export type WorktreeDownResult = { site: string, databaseDropped: string | null, };

export type PruneWorktreesRequest = { 
/**
 * Limit to one project.
 */
projectId?: string | null, dryRun?: boolean, dropDatabases?: boolean, };

export type PruneReport = { dryRun: boolean, 
/**
 * Site ids removed (or that would be, with `dryRun`).
 */
removed: Array<string>, droppedDatabases: Array<string>, 
/**
 * Environments that could not be removed (still registered, for a retry): the others are
 * removed and synced all the same.
 */
failed: Array<PruneFailure>, };

/**
 * One environment `prune` could not remove.
 */
export type PruneFailure = { 
/**
 * Site id (`shop@feature-x`).
 */
site: string, code: ErrorCode, message: string, };

// --- Logs ---

export type LogSource = { 
/**
 * `apache`, `apache-error`, `mysql-8.4`, `mysql-8.4-error`, `redis`,
 * `access:<site>`, `error:<site>`, `php:<site>`.
 */
id: string, file: string, exists: boolean, };

export type LogTailRequest = { source: string, lines: number, sinceUnixMs: number | null, };

export type LogLine = { text: string, atUnixMs: number | null, };

export type LogLines = { source: string, file: string, lines: Array<LogLine>, 
/**
 * Pass back to `read_from` to follow the file.
 */
nextOffset: number, };

/**
 * Lines the log viewer shows (after its filters), saved to `file`.
 */
export type LogExportRequest = { 
/**
 * Absolute path of the file to write (replaced if it exists).
 */
file: string, lines: Array<string>, };

// --- Settings ---

/**
 * One configured service instance (`mysql-8.4`, `redis`).
 */
export type ServiceInstance = { 
/**
 * Service id (`mysql-8.4`).
 */
service: string, 
/**
 * Display label from the kind's descriptor (`MySQL 8.4`); ignored on input.
 */
label: string, 
/**
 * The main port (Mailpit: SMTP).
 */
port: number, alwaysOn: boolean, 
/**
 * Web UI port of kinds that have one (Mailpit), else `null`. On output the effective value
 * (configured, else the default).
 */
uiPort?: number | null, };

/**
 * A registered kind of service, so front-ends offer what the backend knows (no hard-coded
 * list): the project editor lists these, with the configured versions of versioned kinds.
 */
export type ServiceKindInfo = { kind: string, label: string, 
/**
 * A project picks one of `versions`.
 */
versioned: boolean, 
/**
 * Configured series of a versioned kind, settings order (`["8.4", "5.7"]`).
 */
versions: Array<string>, 
/**
 * The version new projects get (first always-on instance), if any.
 */
defaultVersion: string | null, };

export type PortRange = { start: number, end: number, };

export type Settings = { defaultPhpVersion: string | null, tld: string, apachePort: number, 
/**
 * Every other service instance and its port.
 */
services: Array<ServiceInstance>, 
/**
 * Dedicated loopback listeners (worktree environments, loopback-port addresses).
 */
loopbackPortRange: PortRange, closeToTray: boolean, 
/**
 * A running service that exits unexpectedly is started again (up to 3 attempts, 2/4/8 s apart).
 */
autoRestartServices: boolean, 
/**
 * Read-only from the GUI's point of view (set by the first-run wizard).
 */
firstRunCompleted: boolean, 
/**
 * Addresses new projects get when the creator chooses none.
 */
defaultAddressing: Addressing, 
/**
 * New projects get a dedicated MySQL account (`lampo_<id>`, granted only their databases)
 * instead of root. Existing projects keep theirs until switched one by one.
 */
mysqlProjectCredentials: boolean, 
/**
 * Seconds a site's idle php-cgi.exe stays alive (30-3600, default 120): longer keeps RAM
 * busy, shorter means more cold starts.
 */
phpIdleTimeoutSecs: number, };

// --- Diagnostics and shims ---

export type CheckStatus = "ok" | "warning" | "error";

/**
 * One `doctor` check. `id` is stable (`settings`, `project-files`, `vc-runtime`, `ports`,
 * `path-shadowing`, `shims-on-path`, `hosts-block`, `packages`, `services`, `mysql-auth`, `defender`,
 * `stale-worktrees`, `folders`, `app`).
 */
export type DoctorCheck = { id: string, status: CheckStatus, message: string, 
/**
 * Suggested fix (command or action), English.
 */
fix: string | null, };

export type VcRuntimeIssue = { redistributable: string, downloadUrl: string, 
/**
 * Package keys needing it (`php-8.4.1`).
 */
requiredBy: Array<string>, };

export type PortConflict = { port: number, 
/**
 * Service or site wanting the port.
 */
wantedBy: string, owner: string | null, };

/**
 * A tool whose Lampo shim is shadowed by another executable earlier on PATH.
 */
export type ShadowedTool = { 
/**
 * `php`, `composer`, `mysql`, `mysqldump`, `redis-cli`.
 */
tool: string, shim: string, shadowedBy: string, };

export type DoctorReport = { 
/**
 * No check has status `error`.
 */
ok: boolean, checks: Array<DoctorCheck>, missingVcRuntimes: Array<VcRuntimeIssue>, portConflicts: Array<PortConflict>, shadowedTools: Array<ShadowedTool>, 
/**
 * Site ids of worktree environments whose folder is gone.
 */
staleWorktrees: Array<string>, 
/**
 * Package requirements not satisfied (`php 8.2`, `mysql 5.7`).
 */
missingPackages: Array<string>, 
/**
 * Services the plan needs that are `failed` or `stopped` (`apache`, `mysql-8.4`).
 */
servicesDown: Array<string>, hostsFileInSync: boolean, 
/**
 * Project files that do not load, with the reason (`project-files`).
 */
unreadableProjects: Array<UnreadableProject>, };

export type ShimStatus = { shimDir: string, installed: boolean, onPath: boolean, shadowed: Array<ShadowedTool>, };

// --- Integrations ---

export type ClaudeCodeState = "isolated" | "claude-missing" | "not-installed" | "installed" | "outdated";

export type ClaudeCodeStatus = { state: ClaudeCodeState, 
/**
 * First line of `claude --version`.
 */
claudeVersion: string | null, 
/**
 * Version recorded by the last successful install (`.installed-version`).
 */
installedVersion: string | null, 
/**
 * Version of the plugin this app generates now (`<app>-h<hash>`).
 */
expectedVersion: string, 
/**
 * Folder of the generated plugin (a local marketplace).
 */
pluginDir: string, 
/**
 * The `lampo.exe` the hook and the skills call (the copy in the shim dir).
 */
cliPath: string, 
/**
 * Whether that copy exists (installed with the command-line tools).
 */
cliInstalled: boolean, 
/**
 * Last automatic update at startup that failed, cleared by a successful install/update.
 */
updateError: string | null, };

export type StepOutcome = "ok" | "failed" | "ignored";

/**
 * One line of the step log shown in the GUI.
 */
export type ClaudeCodeStep = { 
/**
 * Stable kebab-case id (`detect-claude`, `check-cli`, `write-files`, `validate-plugin`,
 * `validate-marketplace`, `remove-previous-plugin`, `remove-previous-marketplace`,
 * `add-marketplace`, `install-plugin`, `verify`, `write-marker`, `remove-plugin`,
 * `remove-marketplace`, `remove-files`).
 */
step: string, outcome: StepOutcome, 
/**
 * Version, path or the first lines of the program's output.
 */
detail: string | null, };

/**
 * Result of an install/update/removal: never an error for a failing step (the log says which).
 */
export type ClaudeCodeAction = { ok: boolean, steps: Array<ClaudeCodeStep>, status: ClaudeCodeStatus, };

// --- Defender exclusion ---

export type ScanExclusionChange = "add" | "remove";

/**
 * How a request ended: none of these is an error, each is told to the user as it is.
 */
export type ScanExclusionOutcome = "applied" | "cancelled" | "defender-unavailable" | "refused" | "not-confirmed";

/**
 * The result of one request. Defender's exclusions cannot be read without administrator
 * rights, so this is all Lampo knows: there is no "active" state.
 */
export type ScanExclusionResult = { change: ScanExclusionChange, outcome: ScanExclusionOutcome, 
/**
 * The excluded folder (`%LOCALAPPDATA%\Lampo\mysql`).
 */
folder: string, };

/**
 * Which files one request changes, all through one UAC prompt. At least one.
 */
export type DefenderExclusionRequest = { change: ScanExclusionChange, 
/**
 * The files opened by `php-cgi.exe` of every installed PHP and by Apache's `httpd.exe`
 * (process exclusions).
 */
php: boolean, 
/**
 * The MySQL data folder (a path exclusion).
 */
mysql: boolean, 
/**
 * A folder holding the user's projects (a path exclusion): `projectsFolder`, or the
 * suggested one ([`DefenderExclusionInfoDto::projects_folder`]) when it is absent.
 */
projects: boolean, 
/**
 * The projects folder to exclude (an absolute path); only with `projects`.
 */
projectsFolder?: string | null, 
/**
 * The process exclusions Lampo applied to server executables no longer installed (a PHP or
 * Apache uninstalled since): only to remove them, leaving the installed ones excluded.
 */
uninstalled: boolean, };

/**
 * The result of one request (see [`ScanExclusionResultDto`]: there is no "active" state).
 */
export type DefenderExclusionResult = { change: ScanExclusionChange, outcome: ScanExclusionOutcome, 
/**
 * The MySQL data folder, when the request named it.
 */
folder: string | null, 
/**
 * The executables the request named (empty without `php`).
 */
processes: Array<string>, 
/**
 * The projects folder, when the request named it.
 */
projects: string | null, };

/**
 * What each part of a request would cover now, shown before the user asks for it.
 */
export type DefenderExclusionInfo = { 
/**
 * The MySQL data folder (`mysql`).
 */
folder: string, 
/**
 * `php-cgi.exe` of every installed PHP and `httpd.exe` of every installed Apache (`php`).
 */
processes: Array<string>, 
/**
 * The suggested projects folder (`projects`): the deepest folder holding the folders of the
 * registered projects that may be excluded; `null` when there is none.
 */
projectsFolder: string | null, 
/**
 * The executables Lampo excluded that are no longer installed (`uninstalled`, only to
 * remove): a PHP or Apache uninstalled since. Empty when the record of the exclusions does
 * not load.
 */
uninstalledProcesses: Array<string>, };

/**
 * What one Defender exclusion excludes.
 */
export type DefenderExclusionKind = "process" | "path";

/**
 * One exclusion of a request as a PowerShell command, for whoever manages the PC (group policy,
 * Intune) when Defender refuses Lampo's request: the exact path a request would name.
 */
export type DefenderExclusionCommand = { kind: DefenderExclusionKind, path: string, 
/**
 * `Add-MpPreference -ExclusionProcess '<path>'` (or `Remove-MpPreference`, `-ExclusionPath`),
 * one line, the path quoted for PowerShell.
 */
command: string, };

/**
 * A change of the PHP process exclusion Lampo proposes after a PHP is installed or uninstalled,
 * and never makes by itself: only when an exclusion Lampo applied is in place and Microsoft
 * Defender is the active antivirus.
 */
export type DefenderExclusionSuggestion = { change: ScanExclusionChange, 
/**
 * The executables concerned: the installed `php-cgi.exe` not excluded yet (`add`), or the
 * excluded executables no longer installed (`remove`).
 */
processes: Array<string>, 
/**
 * The request making the change (one UAC prompt).
 */
request: DefenderExclusionRequest, 
/**
 * The CLI command making the change.
 */
command: string, };

// --- App ---

export type AppInfo = { version: string, configDir: string, dataDir: string, shimDir: string, controlProtocolVersion: number, };

// --- Event payloads ---

export type ProgressStage = "resolving" | "downloading" | "verifying" | "extracting" | "configuring" | "initializing" | "running";

/**
 * Progress of a long operation (bytes while downloading/extracting, steps otherwise).
 */
export type ProgressReport = { stage: ProgressStage, done: number, total: number | null, subject: string | null, 
/**
 * One output line of a running program (a log entry), when the report carries one.
 */
message?: string | null, };

export type TaskProgressPayload = { taskId: string, stage: ProgressStage, done: number, total: number | null, subject: string | null, 
/**
 * One output line of a running program (a log entry), when the report carries one.
 */
message?: string | null, };

export type TaskOutcome = { "status": "succeeded" } | { "status": "cancelled" } | { "status": "failed", error: AppError, };

export type TaskFinishedPayload = { taskId: string, outcome: TaskOutcome, };

export type ServiceStatusPayload = { service: string, kind: string, state: ServiceState, pid: number | null, port: number | null, message: string | null, };

export type LogStream = "stdout" | "stderr";

export type ServiceLogPayload = { service: string, stream: LogStream, line: string, atUnixMs: number, };

export type ProjectsChangedPayload = { projectIds: Array<string>, };

export type WorktreesChangedPayload = { projectId: string, };

/**
 * One step of the automatic restart of a crashed service:
 * `{ "type": "scheduled", "attempt": 1, "maxAttempts": 3, "delayMs": 2000 }`,
 * `{ "type": "restarted", "attempt": 1 }`, `{ "type": "gave-up", "attempts": 3, "message": "..." }`.
 */
export type AutoRestartStep = { "type": "scheduled", attempt: number, maxAttempts: number, delayMs: number, } | { "type": "restarted", attempt: number, } | { "type": "gave-up", attempts: number, message: string, };

export type ServiceAutoRestartPayload = { service: string, step: AutoRestartStep, };
