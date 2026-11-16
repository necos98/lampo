/** Appends `extra` to `current` keeping only the newest `max` items, with a single copy. */
export function appendCapped<T>(current: readonly T[], extra: readonly T[], max: number): T[] {
	if (extra.length >= max) return extra.slice(extra.length - max);
	const overflow = Math.max(0, current.length + extra.length - max);
	return current.slice(overflow).concat(extra);
}

/**
 * Lines of a live tail, capped at `max`. While following, new lines join `lines` at once; when
 * the reader stops following they wait in a pending queue (also capped) so the visible lines do
 * not shift under the reader, and `pendingCount` says how many arrived meanwhile.
 *
 * `lines` is replaced, never mutated, so a shallow (`$state.raw`) mirror sees every change.
 */
export class TailBuffer<T> {
	readonly #max: number;
	#lines: T[] = [];
	#pending: T[] = [];
	#pendingCount = 0;
	#following = true;

	constructor(max: number) {
		this.#max = max;
	}

	get lines(): readonly T[] {
		return this.#lines;
	}

	get pendingCount(): number {
		return this.#pendingCount;
	}

	get following(): boolean {
		return this.#following;
	}

	/** Replaces everything with a fresh tail; pending lines belong to the old one. */
	reset(lines: readonly T[]): void {
		this.#lines = appendCapped([], lines, this.#max);
		this.#dropPending();
	}

	push(lines: readonly T[]): void {
		if (lines.length === 0) return;
		if (this.#following) {
			this.#lines = appendCapped(this.#lines, lines, this.#max);
		} else {
			this.#pending = appendCapped(this.#pending, lines, this.#max);
			this.#pendingCount += lines.length;
		}
	}

	/** Starts following again, revealing what arrived in the meantime. */
	follow(): void {
		if (this.#following) return;
		this.#following = true;
		if (this.#pending.length) this.#lines = appendCapped(this.#lines, this.#pending, this.#max);
		this.#dropPending();
	}

	unfollow(): void {
		this.#following = false;
	}

	clear(): void {
		this.reset([]);
	}

	#dropPending(): void {
		this.#pending = [];
		this.#pendingCount = 0;
	}
}
