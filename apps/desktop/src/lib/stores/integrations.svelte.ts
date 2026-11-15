import type { ClaudeCodeAction, ClaudeCodeStatus, ClaudeCodeStep } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner } from './runner.js';

/** Coding-agent integrations (Settings > Integrazioni). Only explicit clicks install or remove. */
export class IntegrationsStore {
	readonly claudeCode: Resource<ClaudeCodeStatus>;
	/** `install` covers "Installa" and "Aggiorna". */
	busy = $state<'install' | 'uninstall' | null>(null);
	/** Step log of the last install/update/removal in this window. */
	steps = $state<ClaudeCodeStep[]>([]);
	lastOk = $state<boolean | null>(null);

	readonly #backend: Backend;
	readonly #runner: CommandRunner;

	constructor(backend: Backend, runner: CommandRunner) {
		this.#backend = backend;
		this.#runner = runner;
		this.claudeCode = new Resource(() => backend.commands.claudeCodeStatus());
	}

	loadClaudeCode = () => this.claudeCode.load();

	installClaudeCode = () => this.#act('install', () => this.#backend.commands.installClaudeCode(), m.settings.integrations.claude.installed);
	uninstallClaudeCode = () => this.#act('uninstall', () => this.#backend.commands.uninstallClaudeCode(), m.settings.integrations.claude.removed);

	async #act(kind: 'install' | 'uninstall', op: () => Promise<ClaudeCodeAction>, success: string) {
		this.busy = kind;
		const result = await this.#runner.run(op, { success, warning: (a) => (a.ok ? null : m.settings.integrations.claude.failed) });
		this.busy = null;
		if (result.ok) {
			this.steps = result.value.steps;
			this.lastOk = result.value.ok;
			this.claudeCode.set(result.value.status);
		}
		return result;
	}
}
