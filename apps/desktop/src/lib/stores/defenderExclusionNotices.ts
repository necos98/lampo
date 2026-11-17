import type { DefenderExclusionSuggestion, ScanExclusionChange } from '../api/index.js';
import { isPackageTaskOf } from '../packages/grouping.js';
import type { Notifier } from '../services/notifier.js';
import type { TaskState } from './tasks.svelte.js';

/** The package kind whose install or uninstall may call for a change of the exclusion. */
const PHP_KIND = 'php';

/** The texts of the notices (`messages.settings.defender.php.suggestion`). */
export interface SuggestionTexts {
	add: string;
	remove: string;
	action: Record<ScanExclusionChange, string>;
}

/** What the notices need: the finished tasks, the backend's proposals, and the request a click makes. */
export interface SuggestionSources {
	onTaskFinished(listener: (task: TaskState) => void): () => void;
	suggestions(): Promise<DefenderExclusionSuggestion[]>;
	apply(suggestion: DefenderExclusionSuggestion): unknown;
}

/** Whether `task` installed or uninstalled a PHP successfully. */
export function changedPhp(task: TaskState): boolean {
	const kind = task.meta?.kind;
	return (kind === 'install' || kind === 'uninstall') && task.outcome?.status === 'succeeded' && isPackageTaskOf(task.meta?.key ?? '', PHP_KIND);
}

/**
 * After a PHP is installed or uninstalled, proposes the matching change of the Microsoft Defender
 * PHP exclusion: a notice with a button that makes the request (one UAC prompt). Never makes it by
 * itself; the backend proposes nothing unless Lampo applied the exclusion and Defender is active.
 */
export class DefenderExclusionNotices {
	readonly #sources: SuggestionSources;
	readonly #notifier: Notifier;
	readonly #texts: SuggestionTexts;

	constructor(sources: SuggestionSources, notifier: Notifier, texts: SuggestionTexts) {
		this.#sources = sources;
		this.#notifier = notifier;
		this.#texts = texts;
	}

	start(): Promise<() => void> {
		const stop = this.#sources.onTaskFinished((task) => {
			if (changedPhp(task)) void this.#propose();
		});
		return Promise.resolve(stop);
	}

	async #propose(): Promise<void> {
		let suggestions: DefenderExclusionSuggestion[];
		try {
			suggestions = await this.#sources.suggestions();
		} catch {
			// Advice only: the package change succeeded, and the doctor still reports an uncovered PHP.
			return;
		}
		for (const suggestion of suggestions) {
			const action = { label: this.#texts.action[suggestion.change], run: () => void this.#sources.apply(suggestion) };
			this.#notifier.info(this.#texts[suggestion.change], action);
		}
	}
}
