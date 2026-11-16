import type { AppInfo, DoctorReport, ShimStatus } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner } from './runner.js';

export type ShimAction = 'install' | 'uninstall';

/** Doctor report and CLI shim status. */
export class DiagnosticsStore {
	readonly doctor: Resource<DoctorReport>;
	readonly shims: Resource<ShimStatus>;
	readonly info: Resource<AppInfo>;
	/** The shim action in progress, if any. */
	shimsBusy = $state<ShimAction | null>(null);

	readonly #backend: Backend;
	readonly #runner: CommandRunner;

	constructor(backend: Backend, runner: CommandRunner) {
		this.#backend = backend;
		this.#runner = runner;
		this.doctor = new Resource(() => backend.commands.runDoctor());
		this.shims = new Resource(() => backend.commands.shimStatus());
		this.info = new Resource(() => backend.commands.appInfo());
	}

	runDoctor = () => this.doctor.load();
	loadShims = () => this.shims.load();
	/** Views needing the report only once share one run (the user re-runs it explicitly). */
	ensureDoctor = () => this.doctor.ensure();

	installShims = () => this.#shimAction('install', () => this.#backend.commands.installShims(), m.settings.shims.installed);
	uninstallShims = () => this.#shimAction('uninstall', () => this.#backend.commands.uninstallShims(), m.settings.shims.removed);

	async #shimAction(kind: ShimAction, op: () => Promise<ShimStatus>, success: string) {
		this.shimsBusy = kind;
		const result = await this.#runner.run(op, { success });
		this.shimsBusy = null;
		if (result.ok) {
			this.shims.set(result.value);
			// PATH and shadowing findings changed.
			if (this.doctor.data) void this.doctor.load();
		}
		return result;
	}
}
