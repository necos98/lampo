import { createMatcher, matchPositions, stepMatch, type LogMatcher } from '../logs/search.js';
import type { LogEntry } from './logSession.svelte.js';

/** A request to bring row `index` into view; a new object for every step, even to the same row. */
export interface ScrollRequest {
	index: number;
}

/**
 * Search inside the lines a viewer shows: highlights every match and steps through them. The
 * current match is remembered by line id, not by position, so it survives lines arriving or
 * being trimmed; stepping asks the view to scroll (`scrollRequest`), which works with
 * virtualization because the view computes the row's offset itself.
 */
export class LogSearch {
	readonly #lines: () => readonly LogEntry[];
	readonly #onstep: () => void;

	query = $state('');
	/** Id of the line of the current match, `null` before the first step. */
	currentId = $state<number | null>(null);
	scrollRequest = $state.raw<ScrollRequest | null>(null);

	readonly matcher: LogMatcher | null = $derived(createMatcher(this.query));
	/** Positions in the shown lines of every matching line. */
	readonly matches: readonly number[] = $derived.by(() => matchPositions(this.#lines(), this.matcher));
	/** Index in `matches` of the current match, `-1` when there is none. */
	readonly position: number = $derived.by(() => {
		const lines = this.#lines();
		return this.currentId === null ? -1 : this.matches.findIndex((i) => lines[i].id === this.currentId);
	});

	/** `lines` reads the shown lines; `onstep` runs before every jump (e.g. stop following the tail). */
	constructor(lines: () => readonly LogEntry[], onstep: () => void) {
		this.#lines = lines;
		this.#onstep = onstep;
	}

	next(): void {
		this.#step(1);
	}

	previous(): void {
		this.#step(-1);
	}

	#step(direction: 1 | -1): void {
		if (this.matches.length === 0) return;
		this.#onstep();
		const index = this.matches[stepMatch(this.position, this.matches.length, direction)];
		this.currentId = this.#lines()[index].id;
		this.scrollRequest = { index };
	}
}
