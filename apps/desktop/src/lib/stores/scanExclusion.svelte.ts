import type { DefenderExclusionInfo, DefenderExclusionResult, ScanExclusionChange } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner, Outcome } from './runner.js';

/**
 * The part of Lampo a request excludes: `php` the server processes (`php-cgi.exe` of every
 * installed PHP and Apache's `httpd.exe`), `mysql` the MySQL data folder.
 */
export type DefenderTarget = 'php' | 'mysql';

/** The request in progress, while its UAC prompt is open. */
export interface PendingExclusion {
	target: DefenderTarget;
	change: ScanExclusionChange;
}

/** What the user is told when a request ends: the success of the change, or why nothing changed. */
export function scanExclusionMessage(target: DefenderTarget, result: DefenderExclusionResult): string {
	const t = m.settings.defender;
	return result.outcome === 'applied' ? t[target].done[result.change] : t.outcome[result.outcome];
}

/**
 * The optional Microsoft Defender exclusions (Impostazioni > CLI e integrazioni). Only explicit clicks change
 * them, one request at a time, each through a UAC prompt. Defender's exclusions cannot be read
 * without administrator rights, so the store keeps only the last request made in this window for
 * each target.
 */
export class ScanExclusionStore {
	readonly info: Resource<DefenderExclusionInfo>;
	busy = $state<PendingExclusion | null>(null);
	last = $state<Record<DefenderTarget, DefenderExclusionResult | null>>({ php: null, mysql: null });

	readonly #backend: Backend;
	readonly #runner: CommandRunner;

	constructor(backend: Backend, runner: CommandRunner) {
		this.#backend = backend;
		this.#runner = runner;
		this.info = new Resource(() => backend.commands.defenderExclusionInfo());
	}

	load = () => this.info.load();
	add = (target: DefenderTarget) => this.#change(target, 'add');
	remove = (target: DefenderTarget) => this.#change(target, 'remove');

	/**
	 * `null` while another request is open: its prompt must end first. An outcome other than
	 * `applied` is a warning, not an error: nothing changed and the toast says why.
	 */
	async #change(target: DefenderTarget, change: ScanExclusionChange): Promise<Outcome<DefenderExclusionResult> | null> {
		if (this.busy) return null;
		this.busy = { target, change };
		const request = { change, php: target === 'php', mysql: target === 'mysql' };
		const message = (r: DefenderExclusionResult) => scanExclusionMessage(target, r);
		const result = await this.#runner.run(() => this.#backend.commands.changeDefenderExclusion(request), {
			success: message,
			warning: (r) => (r.outcome === 'applied' ? null : message(r))
		});
		this.busy = null;
		if (result.ok) this.last[target] = result.value;
		return result;
	}
}
