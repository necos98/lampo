import type { ServiceKindInfo, Settings } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner } from './runner.js';

export class SettingsStore {
	readonly value: Resource<Settings>;
	/** The service kinds a project may use (from the backend), with their configured versions. */
	readonly serviceKinds: Resource<ServiceKindInfo[]>;
	saving = $state(false);

	readonly #backend: Backend;
	readonly #runner: CommandRunner;

	constructor(backend: Backend, runner: CommandRunner) {
		this.#backend = backend;
		this.#runner = runner;
		this.value = new Resource(() => backend.commands.getSettings());
		this.serviceKinds = new Resource(() => backend.commands.listServiceKinds());
	}

	load = () => this.value.load();

	async save(next: Settings) {
		this.saving = true;
		const result = await this.#runner.run(() => this.#backend.commands.updateSettings(next), { success: m.settings.saved });
		this.saving = false;
		if (result.ok) {
			this.value.set(result.value);
			void this.serviceKinds.load();
		}
		return result;
	}

	async start(): Promise<() => void> {
		return this.#backend.events.onSettingsChanged(() => {
			void this.value.load();
			void this.serviceKinds.load();
		});
	}
}
