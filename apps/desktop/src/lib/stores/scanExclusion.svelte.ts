import type {
	DefenderExclusionCommand,
	DefenderExclusionInfo,
	DefenderExclusionRequest,
	DefenderExclusionResult,
	DefenderExclusionSuggestion,
	ScanExclusionChange,
	ScanExclusionOutcome
} from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import type { Notifier } from '../services/notifier.js';
import type { CopyText } from './logFiles.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner, Outcome } from './runner.js';

/**
 * The part a request excludes: `php` the server processes (`php-cgi.exe` of every installed PHP
 * and Apache's `httpd.exe`), `mysql` the MySQL data folder, `projects` a folder holding the
 * user's projects.
 */
export type DefenderTarget = 'php' | 'mysql' | 'projects';

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

/** What the user is told when a change proposed after a PHP install or uninstall ends. */
export function suggestionMessage(result: DefenderExclusionResult): string {
	const t = m.settings.defender;
	return result.outcome === 'applied' ? t.php.suggestion.done[result.change] : t.outcome[result.outcome];
}

/** The outcomes that mean a policy controls Defender's exclusions: whoever manages the PC can add them. */
const MANAGED_OUTCOMES: ReadonlySet<ScanExclusionOutcome> = new Set(['refused', 'not-confirmed']);

/** Whether `result` is an addition that whoever manages the PC must make instead (the commands are for them). */
export function needsItCommands(result: DefenderExclusionResult | null): boolean {
	return result?.change === 'add' && MANAGED_OUTCOMES.has(result.outcome);
}

/** The text copied for whoever manages the PC: the PowerShell lines, one per exclusion, as `lampo defender exclusion commands` prints them. */
export function itCommandsText(commands: readonly DefenderExclusionCommand[]): string {
	return commands.map((c) => c.command).join('\n');
}

/** The request naming `target` only; `projectsFolder` is the folder of a `projects` request. */
export function exclusionRequest(target: DefenderTarget, change: ScanExclusionChange, projectsFolder?: string): DefenderExclusionRequest {
	const projects = target === 'projects';
	return { change, php: target === 'php', mysql: target === 'mysql', projects, projectsFolder: projects ? (projectsFolder?.trim() ?? null) : null, uninstalled: false };
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
	last = $state<Record<DefenderTarget, DefenderExclusionResult | null>>({ php: null, mysql: null, projects: null });

	readonly #backend: Backend;
	readonly #runner: CommandRunner;
	readonly #notifier: Notifier;
	readonly #copyText: CopyText;

	constructor(backend: Backend, runner: CommandRunner, notifier: Notifier, copyText: CopyText) {
		this.#backend = backend;
		this.#runner = runner;
		this.#notifier = notifier;
		this.#copyText = copyText;
		this.info = new Resource(() => backend.commands.defenderExclusionInfo());
	}

	load = () => this.info.load();
	/** `projectsFolder`: the folder a `projects` request names (ignored for the others). */
	add = (target: DefenderTarget, projectsFolder?: string) => this.#change(target, 'add', projectsFolder);
	remove = (target: DefenderTarget, projectsFolder?: string) => this.#change(target, 'remove', projectsFolder);

	/** Makes the change proposed after a PHP was installed or uninstalled (one UAC prompt); `null` while another request is open. */
	applySuggestion = (suggestion: DefenderExclusionSuggestion) => this.#request('php', suggestion.change, suggestion.request, suggestionMessage);

	/**
	 * Copies the PowerShell commands adding `target`'s exclusion, for whoever manages the PC: no
	 * prompt, nothing changed. `false` when they could not be built (the error is told) or copied.
	 */
	async copyItCommands(target: DefenderTarget, projectsFolder?: string): Promise<boolean> {
		const request = exclusionRequest(target, 'add', projectsFolder);
		const result = await this.#runner.run(() => this.#backend.commands.defenderExclusionCommands(request));
		if (!result.ok) return false;
		const copied = await this.#copyText(itCommandsText(result.value));
		if (copied) this.#notifier.success(m.settings.defender.itCommands.copied);
		else this.#notifier.error(m.common.copyFailed);
		return copied;
	}

	/**
	 * `null` while another request is open: its prompt must end first. An outcome other than
	 * `applied` is a warning, not an error: nothing changed and the toast says why.
	 */
	#change(target: DefenderTarget, change: ScanExclusionChange, projectsFolder?: string): Promise<Outcome<DefenderExclusionResult> | null> {
		return this.#request(target, change, exclusionRequest(target, change, projectsFolder), (r) => scanExclusionMessage(target, r));
	}

	/** Sends `request` (about `target`) and tells its end with `message`. */
	async #request(
		target: DefenderTarget,
		change: ScanExclusionChange,
		request: DefenderExclusionRequest,
		message: (result: DefenderExclusionResult) => string
	): Promise<Outcome<DefenderExclusionResult> | null> {
		if (this.busy) return null;
		this.busy = { target, change };
		const result = await this.#runner.run(() => this.#backend.commands.changeDefenderExclusion(request), {
			success: message,
			warning: (r) => (r.outcome === 'applied' ? null : message(r))
		});
		this.busy = null;
		if (result.ok) this.last[target] = result.value;
		return result;
	}
}
