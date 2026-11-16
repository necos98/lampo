/**
 * At most one operation per key at a time: a second start while the first is still awaiting is
 * refused instead of sent again (a double click on Installa must not start two installs).
 * `held` stores the keys; pass a `$state` object so `has` is reactive in the UI.
 */
export class InFlight {
	readonly #held: Record<string, true>;

	constructor(held: Record<string, true> = {}) {
		this.#held = held;
	}

	has(key: string): boolean {
		return key in this.#held;
	}

	/** Runs `op` holding `key`; resolves to `null` without calling it when `key` is already held. */
	async run<T>(key: string, op: () => Promise<T>): Promise<T | null> {
		if (this.has(key)) return null;
		this.#held[key] = true;
		try {
			return await op();
		} finally {
			delete this.#held[key];
		}
	}
}
