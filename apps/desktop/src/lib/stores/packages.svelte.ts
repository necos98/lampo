import type { CatalogEntry, InstalledPackage, PackageId, TaskOutcome, TaskStarted, UpdateAvailable } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { buildRows, installTaskKey, uninstallTaskKey, type PackageRow } from '../packages/grouping.js';
import { describeError } from '../services/errors.js';
import type { Notifier } from '../services/notifier.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner, Outcome } from './runner.js';
import type { TasksStore } from './tasks.svelte.js';

export interface PhpOption {
	series: string;
	installed: boolean;
	endOfLife: boolean;
}

/** Catalog, installed packages and available patch updates, plus install/uninstall. */
export class PackagesStore {
	readonly catalog: Resource<CatalogEntry[]>;
	readonly installed: Resource<InstalledPackage[]>;
	readonly updates: Resource<UpdateAvailable[]>;

	readonly #backend: Backend;
	readonly #runner: CommandRunner;
	readonly #tasks: TasksStore;
	readonly #notifier: Notifier;

	constructor(backend: Backend, runner: CommandRunner, tasks: TasksStore, notifier: Notifier) {
		this.#backend = backend;
		this.#runner = runner;
		this.#tasks = tasks;
		this.#notifier = notifier;
		this.catalog = new Resource(() => backend.commands.listCatalog(null));
		this.installed = new Resource(() => backend.commands.listInstalledPackages());
		this.updates = new Resource(() => backend.commands.checkPackageUpdates());
	}

	/** Catalog and installed registry are cheap; the update check may hit the network. */
	loadLocal = () => Promise.all([this.catalog.load(), this.installed.load()]).then(() => undefined);
	checkUpdates = () => this.updates.load();

	get rows(): PackageRow[] {
		if (!this.catalog.data) return [];
		return buildRows(this.catalog.data, this.installed.data ?? [], this.updates.data ?? []);
	}

	rowsOfGroup(group: string): PackageRow[] {
		return this.rows.filter((r) => r.group === group);
	}

	/** PHP minors known to the catalog, with install state: feeds every PHP version picker. */
	get phpOptions(): PhpOption[] {
		return this.rows.filter((r) => r.kind === 'php').map((r) => ({ series: r.series, installed: r.installed !== null, endOfLife: r.endOfLife }));
	}

	/** Extensions shipped by each installed PHP minor, loaded on demand (`null` while loading). */
	phpExtensions = $state<Record<string, string[] | null>>({});

	/**
	 * Loads (once per minor, until packages change) the extension names the installed build of
	 * `phpVersion` can load. Call from an effect; read with `extensionsOf`.
	 */
	loadExtensions(phpVersion: string): void {
		if (phpVersion in this.phpExtensions) return;
		this.phpExtensions[phpVersion] = null;
		void this.#backend.commands.listPhpExtensions(phpVersion).then(
			(list) => (this.phpExtensions[phpVersion] = list.map((e) => e.name)),
			() => undefined // not installed: the editor falls back to the common list
		);
	}

	/** `null` while loading or when that PHP minor is not installed. */
	extensionsOf(phpVersion: string): string[] | null {
		return this.phpExtensions[phpVersion] ?? null;
	}

	taskKey = installTaskKey;

	/** Task keys whose start command is in flight: the task is not tracked yet, but a second click must not start another. */
	#starting = $state<Record<string, true>>({});

	/** True from the click that starts the install until its task finishes. */
	isInstalling(row: Pick<PackageRow, 'kind' | 'series' | 'phpVersion'>): boolean {
		return this.#isBusy(installTaskKey(row.kind, row.series, row.phpVersion));
	}

	/** `null` when an install of the same package is already starting or running. */
	install(row: Pick<PackageRow, 'kind' | 'series' | 'phpVersion'>) {
		const key = installTaskKey(row.kind, row.series, row.phpVersion);
		return this.#startTask('install', key, () => this.#backend.commands.installPackage({ kind: row.kind, series: row.series, phpVersion: row.phpVersion }));
	}

	/** True from the confirmed click until the uninstall task finishes. */
	isUninstalling(id: PackageId): boolean {
		return this.#isBusy(uninstallTaskKey(id));
	}

	/** Starts the uninstall task; its outcome is announced when it finishes (see `start`). `null` if already under way. */
	uninstall(id: PackageId) {
		return this.#startTask('uninstall', uninstallTaskKey(id), () => this.#backend.commands.uninstallPackage(id));
	}

	#isBusy(key: string): boolean {
		return key in this.#starting || this.#tasks.isRunning(key);
	}

	async #startTask(kind: 'install' | 'uninstall', key: string, op: () => Promise<TaskStarted>): Promise<Outcome<TaskStarted> | null> {
		if (this.#isBusy(key)) return null;
		this.#starting[key] = true;
		try {
			const result = await this.#runner.run(op);
			if (result.ok) this.#tasks.track(result.value.taskId, { kind, key });
			return result;
		} finally {
			delete this.#starting[key];
		}
	}

	#announceUninstall(outcome: TaskOutcome | null): void {
		if (outcome?.status === 'succeeded') this.#notifier.success(m.packages.uninstalled);
		else if (outcome?.status === 'failed') this.#notifier.error(describeError(outcome.error));
	}

	async start(): Promise<() => void> {
		const stopEvent = await this.#backend.events.onPackagesChanged(() => {
			this.phpExtensions = {};
			void Promise.all([this.loadLocal(), this.updates.data ? this.updates.load() : undefined]);
		});
		// A finished install or uninstall is the moment the registry changes even if the event is dropped.
		const stopTask = this.#tasks.onFinished((t) => {
			if (t.meta?.kind === 'uninstall') this.#announceUninstall(t.outcome);
			if (t.meta?.kind === 'install' || t.meta?.kind === 'uninstall') void this.loadLocal();
		});
		return () => {
			stopEvent();
			stopTask();
		};
	}
}
