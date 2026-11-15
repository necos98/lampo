import type { Preset } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner } from './runner.js';

export class PresetsStore {
	readonly list: Resource<Preset[]>;

	readonly #backend: Backend;
	readonly #runner: CommandRunner;

	constructor(backend: Backend, runner: CommandRunner) {
		this.#backend = backend;
		this.#runner = runner;
		this.list = new Resource(() => backend.commands.listPresets());
	}

	load = () => this.list.load();

	/** Built-ins are localised through the messages module; custom ones show their own text. */
	label(preset: Preset): { name: string; description: string } {
		const local = preset.builtin ? m.presets[preset.id] : undefined;
		return { name: local?.name ?? preset.name, description: local?.description ?? preset.description };
	}

	async save(projectId: string, name: string, description: string) {
		const result = await this.#runner.run(() => this.#backend.commands.savePreset({ projectId, name, description }), { success: m.presetActions.saved });
		if (result.ok) await this.list.load();
		return result;
	}

	async remove(presetId: string) {
		const result = await this.#runner.run(() => this.#backend.commands.deletePreset(presetId), { success: m.presetActions.deleted });
		if (result.ok) await this.list.load();
		return result;
	}
}
