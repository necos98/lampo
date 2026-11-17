/**
 * Typed wrappers for every Tauri command (`apps/desktop/src-tauri/src/commands`). The GUI never
 * calls `invoke` with raw strings: use these functions. Rejections are `AppError`s.
 *
 * CONTRACT: keep 1:1 with `lampo_command_handler!` in `src-tauri/src/commands/mod.rs`.
 */
import { invoke } from '@tauri-apps/api/core';

import type * as T from './types.js';

/** Command names exactly as registered in Rust. */
export const COMMANDS = {
	appInfo: 'app_info',
	getStatus: 'get_status',
	quitApp: 'quit_app',
	prepareUpdate: 'prepare_update',
	cancelUpdate: 'cancel_update',
	listProjects: 'list_projects',
	getProject: 'get_project',
	detectProject: 'detect_project',
	suggestAddresses: 'suggest_addresses',
	createProject: 'create_project',
	scaffoldProject: 'scaffold_project',
	scaffoldParentExists: 'scaffold_parent_exists',
	updateProject: 'update_project',
	deleteProject: 'delete_project',
	setProjectActive: 'set_project_active',
	setProjectPhp: 'set_project_php',
	relinkProject: 'relink_project',
	exportProject: 'export_project',
	openTerminal: 'open_terminal',
	openProjectFolder: 'open_project_folder',
	openInVsCode: 'open_in_vscode',
	importProject: 'import_project',
	listPresets: 'list_presets',
	savePreset: 'save_preset',
	deletePreset: 'delete_preset',
	listCatalog: 'list_catalog',
	listInstalledPackages: 'list_installed_packages',
	checkPackageUpdates: 'check_package_updates',
	installPackage: 'install_package',
	listPhpExtensions: 'list_php_extensions',
	uninstallPackage: 'uninstall_package',
	firstRunStatus: 'first_run_status',
	runFirstRun: 'run_first_run',
	cancelTask: 'cancel_task',
	listRunningTasks: 'list_running_tasks',
	listServices: 'list_services',
	startServices: 'start_services',
	stopServices: 'stop_services',
	restartServices: 'restart_services',
	applyEnvironment: 'apply_environment',
	listWorktrees: 'list_worktrees',
	createWorktree: 'create_worktree',
	removeWorktree: 'remove_worktree',
	pruneWorktrees: 'prune_worktrees',
	listLogSources: 'list_log_sources',
	tailLog: 'tail_log',
	readLogFrom: 'read_log_from',
	openLogFile: 'open_log_file',
	openLogFolder: 'open_log_folder',
	exportLogLines: 'export_log_lines',
	getSettings: 'get_settings',
	listServiceKinds: 'list_service_kinds',
	updateSettings: 'update_settings',
	runDoctor: 'run_doctor',
	hostsInSync: 'hosts_in_sync',
	shimStatus: 'shim_status',
	installShims: 'install_shims',
	uninstallShims: 'uninstall_shims',
	claudeCodeStatus: 'claude_code_status',
	installClaudeCode: 'install_claude_code',
	uninstallClaudeCode: 'uninstall_claude_code',
	mysqlScanExclusionInfo: 'mysql_scan_exclusion_info',
	changeMysqlScanExclusion: 'change_mysql_scan_exclusion',
	defenderExclusionInfo: 'defender_exclusion_info',
	changeDefenderExclusion: 'change_defender_exclusion'
} as const;

export type CommandName = (typeof COMMANDS)[keyof typeof COMMANDS];

const call = <R>(cmd: CommandName, args?: Record<string, unknown>): Promise<R> => invoke<R>(cmd, args);

// App
export const appInfo = () => call<T.AppInfo>(COMMANDS.appInfo);
export const getStatus = () => call<T.Status>(COMMANDS.getStatus);
export const quitApp = () => call<void>(COMMANDS.quitApp);
/** Stops every service before an update installs (the installer then closes the app). */
export const prepareUpdate = () => call<void>(COMMANDS.prepareUpdate);
/** The update could not start after `prepareUpdate`: services back to the configured state. */
export const cancelUpdate = () => call<void>(COMMANDS.cancelUpdate);

// Projects
export const listProjects = () => call<T.ProjectList>(COMMANDS.listProjects);
export const getProject = (projectId: string) => call<T.Project>(COMMANDS.getProject, { projectId });
export const detectProject = (folder: string) => call<T.Detection>(COMMANDS.detectProject, { folder });
export const suggestAddresses = (projectId: string) => call<T.AddressSuggestions>(COMMANDS.suggestAddresses, { projectId });
export const createProject = (request: T.CreateProjectRequest) => call<T.CreateProjectResult>(COMMANDS.createProject, { request });
/** Background task: clone / Laravel Installer into `<parentDir>\<folderName>`; output lines arrive as progress messages. */
export const scaffoldProject = (request: T.ScaffoldProjectRequest) => call<T.TaskStarted>(COMMANDS.scaffoldProject, { request });
/** Whether `parentDir` is an existing absolute folder a setup can create its project in. */
export const scaffoldParentExists = (parentDir: string) => call<boolean>(COMMANDS.scaffoldParentExists, { parentDir });
export const updateProject = (request: T.UpdateProjectRequest) => call<T.Project>(COMMANDS.updateProject, { request });
export const deleteProject = (projectId: string) => call<T.DeleteProjectResult>(COMMANDS.deleteProject, { projectId });
export const setProjectActive = (request: T.SetProjectActiveRequest) => call<T.Project>(COMMANDS.setProjectActive, { request });
export const setProjectPhp = (request: T.SetProjectPhpRequest) => call<T.Project>(COMMANDS.setProjectPhp, { request });
export const relinkProject = (request: T.RelinkProjectRequest) => call<T.Project>(COMMANDS.relinkProject, { request });
export const exportProject = (request: T.ExportProjectRequest) => call<void>(COMMANDS.exportProject, { request });
export const openTerminal = (projectId: string) => call<void>(COMMANDS.openTerminal, { projectId });
/** Opens the project folder in Windows Explorer; `validation` if the folder is missing. */
export const openProjectFolder = (projectId: string) => call<void>(COMMANDS.openProjectFolder, { projectId });
/** Opens the project folder in VS Code; `package-missing` if VS Code is not installed. */
export const openInVsCode = (projectId: string) => call<void>(COMMANDS.openInVsCode, { projectId });
export const importProject = (request: T.ImportProjectRequest) => call<T.ImportProjectResult>(COMMANDS.importProject, { request });

// Presets
export const listPresets = () => call<T.Preset[]>(COMMANDS.listPresets);
export const savePreset = (request: T.SavePresetRequest) => call<T.Preset>(COMMANDS.savePreset, { request });
export const deletePreset = (presetId: string) => call<void>(COMMANDS.deletePreset, { presetId });

// Packages and tasks
export const listCatalog = (kind: T.PackageKind | null = null) => call<T.CatalogEntry[]>(COMMANDS.listCatalog, { kind });
export const listInstalledPackages = () => call<T.InstalledPackage[]>(COMMANDS.listInstalledPackages);
export const checkPackageUpdates = () => call<T.UpdateAvailable[]>(COMMANDS.checkPackageUpdates);
/** Starts a background task; follow it with `onTaskProgress` / `onTaskFinished`. */
export const installPackage = (request: T.InstallPackageRequest) => call<T.TaskStarted>(COMMANDS.installPackage, { request });
/** Extensions the installed PHP minor (`"8.3"`) can load; `package-missing` if not installed. */
export const listPhpExtensions = (phpVersion: string) => call<T.PhpExtension[]>(COMMANDS.listPhpExtensions, { phpVersion });
/** Starts a background task (services move off the package, then its files are deleted); follow it with `onTaskFinished`. */
export const uninstallPackage = (id: T.PackageId) => call<T.TaskStarted>(COMMANDS.uninstallPackage, { id });
export const firstRunStatus = () => call<T.FirstRunStatus>(COMMANDS.firstRunStatus);
/** Starts a background task. */
export const runFirstRun = () => call<T.TaskStarted>(COMMANDS.runFirstRun);
export const cancelTask = (taskId: string) => call<boolean>(COMMANDS.cancelTask, { taskId });
/** Tasks still running, e.g. started before this window was (re)created. */
export const listRunningTasks = () => call<T.RunningTask[]>(COMMANDS.listRunningTasks);

// Services
export const listServices = () => call<T.ServiceStatus[]>(COMMANDS.listServices);
export const startServices = (request: T.ServicesRequest = { services: [] }) => call<T.SyncReport>(COMMANDS.startServices, { request });
export const stopServices = (request: T.ServicesRequest = { services: [] }) => call<T.SyncReport>(COMMANDS.stopServices, { request });
export const restartServices = (request: T.ServicesRequest = { services: [] }) => call<T.SyncReport>(COMMANDS.restartServices, { request });
export const applyEnvironment = () => call<T.SyncReport>(COMMANDS.applyEnvironment);

// Worktree environments
export const listWorktrees = (projectId: string | null = null) => call<T.WorktreeEnvironment[]>(COMMANDS.listWorktrees, { projectId });
/** Starts a background task; `onWorktreesChanged` fires on success. */
export const createWorktree = (request: T.WorktreeUpRequest) => call<T.TaskStarted>(COMMANDS.createWorktree, { request });
export const removeWorktree = (request: T.WorktreeDownRequest) => call<T.WorktreeDownResult>(COMMANDS.removeWorktree, { request });
export const pruneWorktrees = (request: T.PruneWorktreesRequest = {}) => call<T.PruneReport>(COMMANDS.pruneWorktrees, { request });

// Logs
export const listLogSources = (site: string | null = null) => call<T.LogSource[]>(COMMANDS.listLogSources, { site });
export const tailLog = (request: T.LogTailRequest) => call<T.LogLines>(COMMANDS.tailLog, { request });
export const readLogFrom = (source: string, offset: number) => call<T.LogLines>(COMMANDS.readLogFrom, { source, offset });
export const openLogFile = (source: string) => call<void>(COMMANDS.openLogFile, { source });
export const openLogFolder = (source: string) => call<void>(COMMANDS.openLogFolder, { source });
export const exportLogLines = (request: T.LogExportRequest) => call<void>(COMMANDS.exportLogLines, { request });

// Settings
export const getSettings = () => call<T.Settings>(COMMANDS.getSettings);
export const listServiceKinds = () => call<T.ServiceKindInfo[]>(COMMANDS.listServiceKinds);
export const updateSettings = (settings: T.Settings) => call<T.Settings>(COMMANDS.updateSettings, { settings });

// Diagnostics and shims
export const runDoctor = () => call<T.DoctorReport>(COMMANDS.runDoctor);
/** Only the hosts-file comparison of the doctor, cheap enough to re-check after every change. */
export const hostsInSync = () => call<boolean>(COMMANDS.hostsInSync);
export const shimStatus = () => call<T.ShimStatus>(COMMANDS.shimStatus);
export const installShims = () => call<T.ShimStatus>(COMMANDS.installShims);
export const uninstallShims = () => call<T.ShimStatus>(COMMANDS.uninstallShims);

// Integrations
export const claudeCodeStatus = () => call<T.ClaudeCodeStatus>(COMMANDS.claudeCodeStatus);
export const installClaudeCode = () => call<T.ClaudeCodeAction>(COMMANDS.installClaudeCode);
export const uninstallClaudeCode = () => call<T.ClaudeCodeAction>(COMMANDS.uninstallClaudeCode);

// Microsoft Defender exclusion of the MySQL data folder (a UAC prompt per change)
export const mysqlScanExclusionInfo = () => call<T.ScanExclusionInfo>(COMMANDS.mysqlScanExclusionInfo);
export const changeMysqlScanExclusion = (change: T.ScanExclusionChange) => call<T.ScanExclusionResult>(COMMANDS.changeMysqlScanExclusion, { change });

// Microsoft Defender exclusions of the PHP/Apache servers and/or the MySQL data folder (one UAC prompt per change)
export const defenderExclusionInfo = () => call<T.DefenderExclusionInfo>(COMMANDS.defenderExclusionInfo);
export const changeDefenderExclusion = (request: T.DefenderExclusionRequest) => call<T.DefenderExclusionResult>(COMMANDS.changeDefenderExclusion, { request });
