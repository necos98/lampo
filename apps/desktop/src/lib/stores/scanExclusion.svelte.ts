import type { ScanExclusionChange, ScanExclusionInfo, ScanExclusionResult } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner } from './runner.js';

/** What the user is told when a request ends: the success of the change, or why nothing changed. */
export function scanExclusionMessage(result: ScanExclusionResult): string {
	const t = m.settings.defender;
	return result.outcome === 'applied' ? t.done[result.change] : t.outcome[result.outcome];
}

/**
 * The Microsoft Defender exclusion of the MySQL data folder (Settings > Strumenti). Only explicit
 * clicks change it, each through a UAC prompt. Defender's exclusions cannot be read without
 * administrator rights, so the store keeps only the last request made in this window.
 */
export class ScanExclusionStore {
	readonly info: Resource<ScanExclusionInfo>;
	busy = $state<ScanExclusionChange | null>(null);
	last = $state<ScanExclusionResult | null>(null);

	readonly #backend: Backend;
	readonly #runner: CommandRunner;

	constructor(backend: Backend, runner: CommandRunner) {
		this.#backend = backend;
		this.#runner = runner;
		this.info = new Resource(() => backend.commands.mysqlScanExclusionInfo());
	}

	load = () => this.info.load();
	add = () => this.#change('add');
	remove = () => this.#change('remove');

	/** An outcome other than `applied` is a warning, not an error: nothing changed and the toast says why. */
	async #change(change: ScanExclusionChange) {
		this.busy = change;
		const result = await this.#runner.run(() => this.#backend.commands.changeMysqlScanExclusion(change), {
			success: scanExclusionMessage,
			warning: (r) => (r.outcome === 'applied' ? null : scanExclusionMessage(r))
		});
		this.busy = null;
		if (result.ok) this.last = result.value;
		return result;
	}
}
