import type { AppInfo, DoctorReport, ShimStatus } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner } from './runner.js';

/** Doctor report and CLI shim status. */
export class DiagnosticsStore {
	readonly doctor: Resource<DoctorReport>;
	readonly shims: Resource<ShimStatus>;
	readonly info: Resource<AppInfo>;
	installingShims = $state(false);

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

	async installShims() {
		this.installingShims = true;
		const result = await this.#runner.run(() => this.#backend.commands.installShims(), { success: m.settings.shims.installed });
		this.installingShims = false;
		if (result.ok) {
			this.shims.set(result.value);
			// PATH and shadowing findings changed.
			if (this.doctor.data) void this.doctor.load();
		}
		return result;
	}
}
