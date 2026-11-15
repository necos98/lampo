import type { CatalogEntry, InstalledPackage, PackageId, TaskOutcome, UpdateAvailable } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { buildRows, installTaskKey, uninstallTaskKey, type PackageRow } from '../packages/grouping.js';
import { describeError } from '../services/errors.js';
import type { Notifier } from '../services/notifier.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner } from './runner.js';
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

	isInstalling(row: Pick<PackageRow, 'kind' | 'series' | 'phpVersion'>): boolean {
		return this.#tasks.isRunning(installTaskKey(row.kind, row.series, row.phpVersion));
	}

	async install(row: Pick<PackageRow, 'kind' | 'series' | 'phpVersion'>) {
		const result = await this.#runner.run(() => this.#backend.commands.installPackage({ kind: row.kind, series: row.series, phpVersion: row.phpVersion }));
		if (result.ok) this.#tasks.track(result.value.taskId, { kind: 'install', key: installTaskKey(row.kind, row.series, row.phpVersion) });
		return result;
	}

	isUninstalling(id: PackageId): boolean {
		return this.#tasks.isRunning(uninstallTaskKey(id));
	}

	/** Starts the uninstall task; its outcome is announced when it finishes (see `start`). */
	async uninstall(id: PackageId) {
		const result = await this.#runner.run(() => this.#backend.commands.uninstallPackage(id));
		if (result.ok) this.#tasks.track(result.value.taskId, { kind: 'uninstall', key: uninstallTaskKey(id) });
		return result;
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
