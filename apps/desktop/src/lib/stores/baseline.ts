/**
 * The "last saved" snapshot of a draft. A draft is dirty when its current value no longer matches
 * the baseline, and a backend value is stale news (changed elsewhere) when it does not match either.
 * Values are compared by their JSON form, so they must be plain data with a stable key order.
 */
export class Baseline {
	#snapshot = '';

	set(value: unknown): void {
		this.#snapshot = JSON.stringify(value);
	}

	matches(value: unknown): boolean {
		return JSON.stringify(value) === this.#snapshot;
	}
}
