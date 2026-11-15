/**
 * Composition root of the GUI state. The only place that picks the backend and wires stores
 * together; views import the singletons from here and never touch `invoke` or event names.
 */
import { backend } from '../backend/gateway.js';
import { notifier } from '../services/notifier.js';
import { DiagnosticsStore } from './diagnostics.svelte.js';
import { ElevationStore } from './elevation.svelte.js';
import { FirstRunStore } from './firstRun.svelte.js';
import { LogSession } from './logSession.svelte.js';
import { LogSourcesStore } from './logSources.svelte.js';
import { PackagesStore } from './packages.svelte.js';
import { PresetsStore } from './presets.svelte.js';
import { ProjectsStore } from './projects.svelte.js';
import { CommandRunner } from './runner.js';
import { ServicesStore } from './services.svelte.js';
import { SettingsStore } from './settings.svelte.js';
import { TasksStore } from './tasks.svelte.js';
import { UpdaterStore } from './updater.svelte.js';
import { WorktreesStore } from './worktrees.svelte.js';
import { CreateProjectFlow } from './createProject.svelte.js';

export const simulated = backend.simulated;

export const elevation = new ElevationStore(backend, notifier);
const runner = new CommandRunner(notifier, () => elevation.flag());

export const tasks = new TasksStore(backend);
export const projects = new ProjectsStore(backend, runner);
export const presets = new PresetsStore(backend, runner);
export const services = new ServicesStore(backend, runner);
export const packages = new PackagesStore(backend, runner, tasks);
export const worktrees = new WorktreesStore(backend, runner, tasks);
export const settings = new SettingsStore(backend, runner);
export const diagnostics = new DiagnosticsStore(backend, runner);
export const firstRun = new FirstRunStore(backend, runner, tasks);
export const logSources = new LogSourcesStore(backend);
export const updater = new UpdaterStore(backend);

/** Per-visit objects are created through factories so they get the shared backend and stores. */
export const createLogSession = () => new LogSession(backend);
export const createProjectFlow = () => new CreateProjectFlow(backend, projects, tasks);

interface Startable {
	start(): Promise<() => void>;
}

/**
 * Subscribes every store to its events (once) and loads the state the shell needs. Everything is
 * re-derived from the backend, because the window is destroyed when the app goes to the tray.
 * Returns a disposer that removes all listeners.
 */
export async function startStores(): Promise<() => void> {
	const startables: Startable[] = [elevation, tasks, projects, services, packages, worktrees, settings, firstRun];
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
