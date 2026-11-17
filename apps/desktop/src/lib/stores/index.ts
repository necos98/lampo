/**
 * Composition root of the GUI state. The only place that picks the backend and wires stores
 * together; views import the singletons from here and never touch `invoke` or event names.
 */
import { goto } from '$app/navigation';

import { backend } from '../backend/gateway.js';
import { messages as m } from '../messages/index.js';
import { notifier } from '../services/notifier.js';
import { copyText } from '../services/platform.js';
import { AutoRestartNotices } from './autoRestartNotices.js';
import { DefenderExclusionNotices } from './defenderExclusionNotices.js';
import { DiagnosticsStore } from './diagnostics.svelte.js';
import { ElevationStore } from './elevation.svelte.js';
import { FirstRunStore } from './firstRun.svelte.js';
import { IntegrationsStore } from './integrations.svelte.js';
import { LogFileActions } from './logFiles.js';
import { LogSearch } from './logSearch.svelte.js';
import { LogSession } from './logSession.svelte.js';
import { LogSourcesStore } from './logSources.svelte.js';
import { PackagesStore } from './packages.svelte.js';
import { PresetsStore } from './presets.svelte.js';
import { ProjectsStore } from './projects.svelte.js';
import { CommandRunner } from './runner.js';
import { ServicesStore } from './services.svelte.js';
import { SettingsStore } from './settings.svelte.js';
import { TasksStore } from './tasks.svelte.js';
import { UnsavedChanges } from './unsavedChanges.js';
import { UpdaterStore } from './updater.svelte.js';
import { WorktreesStore } from './worktrees.svelte.js';
import { CreateProjectFlow } from './createProject.svelte.js';
import { ScaffoldSession } from './scaffoldSession.js';
import { ScanExclusionStore } from './scanExclusion.svelte.js';

/** The create screen, where a scaffolded folder is confirmed as a project. */
const CREATE_PROJECT_ROUTE = '/projects/new';

export const simulated = backend.simulated;

const runner = new CommandRunner(notifier, () => elevation.flag(), (href) => void goto(href));
export const elevation = new ElevationStore(backend, runner);

export const tasks = new TasksStore(backend);
export const projects = new ProjectsStore(backend, runner);
export const presets = new PresetsStore(backend, runner);
export const services = new ServicesStore(backend, runner);
export const packages = new PackagesStore(backend, runner, tasks, notifier);
export const worktrees = new WorktreesStore(backend, runner, tasks);
export const settings = new SettingsStore(backend, runner);
export const diagnostics = new DiagnosticsStore(backend, runner);
export const integrations = new IntegrationsStore(backend, runner);
export const scanExclusion = new ScanExclusionStore(backend, runner, notifier, copyText);
export const firstRun = new FirstRunStore(backend, runner, tasks);
export const logSources = new LogSourcesStore(backend);
export const logFiles = new LogFileActions(backend, runner, notifier, copyText);
export const updater = new UpdaterStore(backend);
/** Each step of the automatic restart of a crashed service, announced from anywhere. */
const autoRestartNotices = new AutoRestartNotices(
	backend.events,
	notifier,
	m.services.autoRestart,
	(service) => services.list.data?.find((s) => s.service === service)?.label ?? service
);
/** After a PHP is installed or uninstalled, the proposed change of the Defender exclusion, made only on a click. */
const defenderExclusionNotices = new DefenderExclusionNotices(
	{ onTaskFinished: (listener) => tasks.onFinished(listener), suggestions: () => backend.commands.defenderExclusionSuggestions(), apply: (s) => scanExclusion.applySuggestion(s) },
	notifier,
	m.settings.defender.php.suggestion
);
/** A new-project setup survives leaving the create screen; its end is announced from anywhere. */
const scaffoldSession = new ScaffoldSession(tasks, () =>
	notifier.success(m.create.readyElsewhere, { label: m.create.register, run: () => void goto(CREATE_PROJECT_ROUTE) })
);
/** The draft forms on screen, so the shell can guard navigation and route Ctrl+S to them. */
export const unsavedChanges = new UnsavedChanges();

/** Per-visit objects are created through factories so they get the shared backend and stores. */
export const createLogSession = () => new LogSession(backend);
/** Search over what `session` shows; a jump stops following the tail so the match stays in view. */
export const createLogSearch = (session: LogSession) =>
	new LogSearch(
		() => session.visible,
		() => session.unfollow()
	);
export const createProjectFlow = () => new CreateProjectFlow(backend, projects, runner, tasks, scaffoldSession, () => settings.value.data?.defaultAddressing);

interface Startable {
	start(): Promise<() => void>;
}

/**
 * Subscribes every store to its events (once) and loads the state the shell needs. Everything is
 * re-derived from the backend, because the window is destroyed when the app goes to the tray.
 * Returns a disposer that removes all listeners.
 */
export async function startStores(): Promise<() => void> {
	const startables: Startable[] = [elevation, tasks, projects, services, packages, worktrees, settings, firstRun, scaffoldSession, autoRestartNotices, defenderExclusionNotices];
	// Subscribe before loading, so nothing that happens during the first load is missed.
	const stops = await Promise.all(startables.map((s) => s.start()));
	void firstRun.load();
	void services.load();
	void projects.load();
	void presets.load();
	void settings.load();
	void worktrees.load();
	void elevation.refresh();
	return () => stops.forEach((stop) => stop());
}
