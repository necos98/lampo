import type { CatalogEntry, InstalledPackage, PackageId, UpdateAvailable } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { buildRows, installTaskKey, type PackageRow } from '../packages/grouping.js';
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
	/** Package ids with an uninstall in flight. */
	uninstalling = $state<Record<string, boolean>>({});

	readonly #backend: Backend;
	readonly #runner: CommandRunner;
	readonly #tasks: TasksStore;

	constructor(backend: Backend, runner: CommandRunner, tasks: TasksStore) {
		this.#backend = backend;
		this.#runner = runner;
		this.#tasks = tasks;
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

	async uninstall(id: PackageId) {
		const key = `${id.kind}-${id.version}-${id.phpVersion ?? ''}`;
		this.uninstalling[key] = true;
		const result = await this.#runner.run(() => this.#backend.commands.uninstallPackage(id), { success: m.packages.uninstalled });
		delete this.uninstalling[key];
		if (result.ok) await this.loadLocal();
		return result;
	}

	async start(): Promise<() => void> {
		const stopEvent = await this.#backend.events.onPackagesChanged(() => {
			this.phpExtensions = {};
			void Promise.all([this.loadLocal(), this.updates.data ? this.updates.load() : undefined]);
		});
		// A finished install is the moment the registry changes even if the event is dropped.
		const stopTask = this.#tasks.onFinished((t) => {
			if (t.meta?.kind === 'install') void this.loadLocal();
		});
		return () => {
			stopEvent();
			stopTask();
		};
	}
}
