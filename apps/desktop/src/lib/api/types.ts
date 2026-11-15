/**
 * Hand-written mirror of the Rust DTOs in `crates/lampo-application/src/dto`, `events.rs` and
 * `error.rs`. Wire format: camelCase fields, kebab-case enum strings.
 *
 * CONTRACT: change this file together with the Rust DTOs (same PR).
 */

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

export type ErrorCode =
	| 'validation'
	| 'not-found'
	| 'conflict'
	| 'project-not-resolved'
	| 'elevation-required'
	| 'elevation-denied'
	| 'app-unavailable'
	| 'package-missing'
	| 'network'
	| 'checksum-mismatch'
	| 'config-rejected'
	| 'port-in-use'
	| 'process-failed'
	| 'io'
	| 'cancelled'
	| 'timeout'
	| 'unsupported'
	| 'protocol-mismatch'
	| 'internal';

/** Rejection value of every command promise. */
export interface AppError {
	code: ErrorCode;
	/** English, developer-facing. Show `messages.errors[code]` to users. */
	message: string;
	/** e.g. `{ output }` for `config-rejected`, `{ port, owner }` for `port-in-use`. */
	details?: unknown;
}

// ---------------------------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------------------------

export type SiteAddress =
	| { kind: 'hostname'; host: string }
	| { kind: 'localhost'; host: string }
	| { kind: 'loopback-port'; port: number };

export type Addressing = 'standard' | 'no-elevation';
export type TlsMode = 'disabled' | 'local-ca';

export interface ProjectEnvironment {
	/** PHP minor, e.g. "8.3". */
	phpVersion: string;
	extensions: string[];
	/** Add-on extension package kinds: "xdebug", "php-redis". */
	addons: string[];
	phpIni: Record<string, string>;
	/** Relative to the project folder; "" = the folder itself. */
	publicDir: string;
	/** Services the project needs while served (one entry per kind). MySQL also holds its database. */
	services: ServiceUse[];
	apacheDirectives: string;
}

/** One service a project uses: `{ kind: 'mysql', version: '8.4' }`, `{ kind: 'redis', version: null }`. */
export interface ServiceUse {
	kind: string;
	/** Series for versioned kinds (see `ServiceKindInfo.versioned`), else null. */
	version: string | null;
}

/** Addresses for a project id from the backend's rules and port allocator (`suggest_addresses`). */
export interface AddressSuggestions {
	hostname: SiteAddress;
	localhost: SiteAddress;
	/** The free loopback port the allocator would assign now (not reserved). */
	loopbackPort: SiteAddress;
	/** Localhost + loopback port: never needs the UAC prompt (the default). */
	noElevation: SiteAddress[];
	/** Hosts-file name + localhost. */
	standard: SiteAddress[];
}

export interface DatabaseConnection {
	mysqlVersion: string;
	host: string;
	port: number;
	user: string;
	password: string;
	database: string;
}

export interface Project {
	id: string;
	name: string;
	root: string;
	folderExists: boolean;
	addresses: SiteAddress[];
	url: string;
	urls: string[];
	documentRoot: string;
	environment: ProjectEnvironment;
	databaseName: string | null;
	database: DatabaseConnection | null;
	tls: TlsMode;
	active: boolean;
	originPreset: string | null;
	worktrees: WorktreeEnvironment[];
}

export interface ProjectSummary {
	id: string;
	name: string;
	root: string;
	active: boolean;
	url: string;
	phpVersion: string;
	mysqlVersion: string | null;
	worktreeCount: number;
}

/** Only `root` is required. */
export interface CreateProjectRequest {
	root: string;
	name?: string | null;
	id?: string | null;
	presetId?: string | null;
	phpVersion?: string | null;
	environment?: ProjectEnvironment | null;
	addresses?: SiteAddress[] | null;
	addressing?: Addressing;
	activate?: boolean;
	/** Create the project's MySQL database (if it uses MySQL). Default `true`. */
	createDatabase?: boolean;
}

export type DatabaseSetupStatus = 'created' | 'already-exists' | 'package-missing' | 'failed';

/** What happened to a new project's database; warn on `package-missing` / `failed`. */
export interface DatabaseSetup {
	name: string;
	status: DatabaseSetupStatus;
	/** Why, for `package-missing` and `failed`. */
	message: string | null;
}

/** `createProject` result: the project plus, when a database was attempted, its outcome. */
export interface CreateProjectResult extends Project {
	/** Absent when nothing was attempted (no MySQL, or `createDatabase: false`). */
	databaseSetup?: DatabaseSetup;
}

// --- New project setup (`scaffold_project`) ---

export type StarterKit = 'none' | 'react' | 'vue' | 'livewire' | 'svelte';
export type TestingFramework = 'pest' | 'phpunit';

export type ScaffoldSource =
	| { kind: 'git-clone'; url: string; branch?: string | null }
	| {
			kind: 'laravel';
			/** PHP minor running the installer; default: newest installed. */
			phpVersion?: string | null;
			starterKit?: StarterKit;
			testing?: TestingFramework;
			/** `--git`: initialise a repository (needs git). */
			git?: boolean;
			/** `--npm`: install and build the front-end (needs Node.js). */
			npm?: boolean;
	  };

/** Creates `<parentDir>\<folderName>` (background task). */
export interface ScaffoldProjectRequest {
	parentDir: string;
	/** Required for Laravel; for a clone it defaults to the repository name. */
	folderName?: string | null;
	source: ScaffoldSource;
}

/** Result of the setup (CLI/control pipe; the GUI task only reports its outcome). */
export interface ScaffoldProjectResult {
	folder: string;
	/** PHP minor the Laravel Installer ran with, `null` for a clone. */
	phpVersion: string | null;
}

export interface UpdateProjectRequest {
	projectId: string;
	name: string;
	addresses: SiteAddress[];
	environment: ProjectEnvironment;
	databaseName: string | null;
	tls: TlsMode;
}

export interface SetProjectActiveRequest {
	projectId: string;
	active: boolean;
}

export interface SetProjectPhpRequest {
	projectId: string;
	phpVersion: string;
}

export interface RelinkProjectRequest {
	projectId: string;
	root: string;
}

export interface ExportProjectRequest {
	projectId: string;
	file: string;
}

export interface ImportProjectRequest {
	file: string;
	root: string | null;
}

export interface Detection {
	folder: string;
	projectType: string | null;
	presetId: string | null;
	phpVersion: string | null;
	suggestedName: string;
	suggestedId: string;
	evidence: string[];
	/** Service kinds the folder's own files use (`mysql`, `redis`; from a Docker Compose file). */
	services: string[];
	/** What Lampo cannot provide for this folder (PostgreSQL, Mailpit…), in English. */
	warnings: string[];
	alreadyRegistered: string | null;
}

// ---------------------------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------------------------

export interface Preset {
	id: string;
	/** English fallback; localise built-ins via `messages.presets[id]`. */
	name: string;
	description: string;
	builtin: boolean;
	environment: ProjectEnvironment;
}

export interface SavePresetRequest {
	projectId: string;
	name: string;
	description?: string;
}

// ---------------------------------------------------------------------------------------------
// Packages and tasks
// ---------------------------------------------------------------------------------------------

/** "php" | "apache" | "mod-fcgid" | "mysql" | "redis" | "composer" | "xdebug" | "php-redis" | … */
export type PackageKind = string;

export interface PackageId {
	kind: PackageKind;
	version: string;
	phpVersion: string | null;
}

export type CatalogChannel = 'frozen' | 'live';

export interface CatalogEntry {
	id: PackageId;
	channel: CatalogChannel;
	endOfLife: boolean;
	sizeBytes: number | null;
	vcRuntime: string | null;
	vcRuntimeInstalled: boolean | null;
	installed: boolean;
	/** `<major>.<minor>` of the version, computed by the backend. */
	series: string;
	/** Tab to list it under: "php", a service kind ("mysql", "redis", "apache"), "addons", "tools", or any future group. */
	group: string;
}

export interface InstalledPackage {
	id: PackageId;
	path: string;
	usedBy: string[];
	series: string;
}

export interface InstallPackageRequest {
	kind: PackageKind;
	version?: string | null;
	series?: string | null;
	phpVersion?: string | null;
}

export interface UpdateAvailable {
	kind: PackageKind;
	series: string;
	phpVersion: string | null;
	installed: string;
	latest: string;
}

export interface TaskStarted {
	taskId: string;
}

/** An extension an installed PHP minor can load (from its `ext` folder). */
export interface PhpExtension {
	name: string;
	/** Loaded with `zend_extension` (opcache). */
	zend: boolean;
}

/** What a background task does (as requested when it was started). */
export type TaskOperation =
	| { kind: 'install-package'; request: InstallPackageRequest }
	| { kind: 'first-run' }
	| { kind: 'worktree-up'; request: WorktreeUpRequest }
	| { kind: 'scaffold-project'; request: ScaffoldProjectRequest };

/** A background task that has not finished yet. */
export interface RunningTask {
	taskId: string;
	operation: TaskOperation;
	/** Latest progress (stage, bytes), if any arrived yet. */
	progress: ProgressReport | null;
	/** Latest progress per subject (a first run installs several packages). */
	subjects: ProgressReport[];
}

export interface FirstRunItem {
	kind: PackageKind;
	series: string | null;
	installed: boolean;
}

export interface FirstRunStatus {
	completed: boolean;
	items: FirstRunItem[];
}

// ---------------------------------------------------------------------------------------------
// Services
// ---------------------------------------------------------------------------------------------

export type ServiceState = 'stopped' | 'starting' | 'running' | 'stopping' | 'failed' | 'unknown';

export interface ServiceStatus {
	/** "apache" | "redis" | "mysql-8.4" … */
	service: string;
	/** Service kind ("apache", "mysql", "redis", or any kind the backend registers). */
	kind: string;
	/** Display label from the backend ("MySQL 8.4"). */
	label: string;
	state: ServiceState;
	pid: number | null;
	port: number | null;
	message: string | null;
	required: boolean;
	requiredBy: string[];
}

export interface ServicesRequest {
	/** Empty = every planned service. */
	services: string[];
}

export interface SyncReport {
	started: string[];
	stopped: string[];
	restarted: string[];
	hostsFileUpdated: boolean;
	warnings: string[];
}

export interface Status {
	appRunning: boolean;
	services: ServiceStatus[];
	projects: ProjectSummary[];
	worktreeCount: number;
}

// ---------------------------------------------------------------------------------------------
// Worktree environments
// ---------------------------------------------------------------------------------------------

export type DatabaseMode = { mode: 'shared' } | { mode: 'fresh' } | { mode: 'clone'; source: string };

export interface WorktreeEnvironment {
	/** "shop@feature-login" */
	site: string;
	projectId: string;
	slug: string;
	root: string;
	branch: string | null;
	/** Canonical URL: http://127.0.0.1:<port> */
	url: string;
	urls: string[];
	loopbackPort: number;
	localhostAlias: string;
	documentRoot: string;
	databaseMode: DatabaseMode;
	database: DatabaseConnection | null;
	stale: boolean;
	createdAtUnixMs: number;
}

export interface WorktreeUpRequest {
	path: string;
	projectId?: string | null;
	database?: DatabaseMode;
}

export interface WorktreeUpResult {
	environment: WorktreeEnvironment;
	created: boolean;
}

export interface WorktreeDownRequest {
	/** Worktree folder or site id. */
	target: string;
	dropDatabase?: boolean;
}

export interface WorktreeDownResult {
	site: string;
	databaseDropped: string | null;
}

export interface PruneWorktreesRequest {
	projectId?: string | null;
	dryRun?: boolean;
	dropDatabases?: boolean;
}

export interface PruneReport {
	dryRun: boolean;
	removed: string[];
	droppedDatabases: string[];
}

// ---------------------------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------------------------

export interface LogSource {
	id: string;
	file: string;
	exists: boolean;
}

export interface LogTailRequest {
	source: string;
	lines: number;
	sinceUnixMs: number | null;
}

export interface LogLine {
	text: string;
	atUnixMs: number | null;
}

export interface LogLines {
	source: string;
	file: string;
	lines: LogLine[];
	nextOffset: number;
}

// ---------------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------------

/** One configured service instance ("mysql-8.4", "redis"). */
export interface ServiceInstance {
	service: string;
	/** "MySQL 8.4"; ignored when saving. */
	label: string;
	port: number;
	alwaysOn: boolean;
}

/** A service kind the backend registers (`list_service_kinds`), for the project editor. */
export interface ServiceKindInfo {
	kind: string;
	label: string;
	/** A project picks one of `versions`. */
	versioned: boolean;
	/** Configured series, settings order. */
	versions: string[];
	defaultVersion: string | null;
}

export interface PortRange {
	start: number;
	end: number;
}

export interface Settings {
	defaultPhpVersion: string | null;
	tld: string;
	apachePort: number;
	/** Every service instance other than Apache, with its port. */
	services: ServiceInstance[];
	loopbackPortRange: PortRange;
	closeToTray: boolean;
	firstRunCompleted: boolean;
}

// ---------------------------------------------------------------------------------------------
// Diagnostics, shims, app
// ---------------------------------------------------------------------------------------------

export type CheckStatus = 'ok' | 'warning' | 'error';

export interface DoctorCheck {
	id: string;
	status: CheckStatus;
	message: string;
	fix: string | null;
}

export interface VcRuntimeIssue {
	redistributable: string;
	downloadUrl: string;
	requiredBy: string[];
}

export interface PortConflict {
	port: number;
	wantedBy: string;
	owner: string | null;
}

export interface ShadowedTool {
	tool: string;
	shim: string;
	shadowedBy: string;
}

export interface DoctorReport {
	ok: boolean;
	checks: DoctorCheck[];
	missingVcRuntimes: VcRuntimeIssue[];
	portConflicts: PortConflict[];
	shadowedTools: ShadowedTool[];
	staleWorktrees: string[];
	missingPackages: string[];
	hostsFileInSync: boolean;
}

export interface ShimStatus {
	shimDir: string;
	installed: boolean;
	onPath: boolean;
	shadowed: ShadowedTool[];
}

// Integrations (Settings > Integrazioni)

/** `isolated`: isolated data root (`LAMPO_HOME`), the real Claude Code is never touched. */
export type ClaudeCodeState = 'isolated' | 'claude-missing' | 'not-installed' | 'installed' | 'outdated';

export interface ClaudeCodeStatus {
	state: ClaudeCodeState;
	claudeVersion: string | null;
	installedVersion: string | null;
	expectedVersion: string;
	pluginDir: string;
	cliPath: string;
	cliInstalled: boolean;
	/** Last automatic update at startup that failed. */
	updateError: string | null;
}

export type StepOutcome = 'ok' | 'failed' | 'ignored';

export interface ClaudeCodeStep {
	/** Stable id: `detect-claude`, `check-cli`, `write-files`, `validate-plugin`, ... */
	step: string;
	outcome: StepOutcome;
	detail: string | null;
}

export interface ClaudeCodeAction {
	ok: boolean;
	steps: ClaudeCodeStep[];
	status: ClaudeCodeStatus;
}

export interface AppInfo {
	version: string;
	configDir: string;
	dataDir: string;
	shimDir: string;
	controlProtocolVersion: number;
}

// ---------------------------------------------------------------------------------------------
// Event payloads
// ---------------------------------------------------------------------------------------------

export type ProgressStage = 'resolving' | 'downloading' | 'verifying' | 'extracting' | 'configuring' | 'initializing' | 'running';

export interface TaskProgressPayload {
	taskId: string;
	stage: ProgressStage;
	done: number;
	total: number | null;
	subject: string | null;
	/** One output line of a running program (`running` stage), shown in a log. */
	message?: string | null;
}

/** A progress report without its task id (`ProgressDto`). */
export type ProgressReport = Omit<TaskProgressPayload, 'taskId'>;

export type TaskOutcome ={ status: 'succeeded' } | { status: 'cancelled' } | { status: 'failed'; error: AppError };

export interface TaskFinishedPayload {
	taskId: string;
	outcome: TaskOutcome;
}

export interface ServiceStatusPayload {
	service: string;
	/** Service kind ("apache", "mysql", "redis", or any kind the backend registers). */
	kind: string;
	state: Exclude<ServiceState, 'unknown'>;
	pid: number | null;
	port: number | null;
	message: string | null;
}

export interface ServiceLogPayload {
	service: string;
	stream: 'stdout' | 'stderr';
	line: string;
	atUnixMs: number;
}

export interface ProjectsChangedPayload {
	projectIds: string[];
}

export interface WorktreesChangedPayload {
	projectId: string;
}
