import { toAppError, type AppError } from '../api/index.js';

export type ResourceStatus = 'idle' | 'loading' | 'ready' | 'error';

/**
 * A remotely owned value with loading/error state. Every load and every local patch bumps a
 * generation counter, so a slow response that started before a newer event or response can never
 * overwrite fresher data (events may arrive out of order with command results).
 */
export class Resource<T> {
	data = $state<T | null>(null);
	status = $state<ResourceStatus>('idle');
	error = $state<AppError | null>(null);
	/** A reload is running while `data` is already shown. */
	refreshing = $state(false);

	#generation = 0;
	readonly #fetcher: () => Promise<T>;

	constructor(fetcher: () => Promise<T>) {
		this.#fetcher = fetcher;
	}

	get loading(): boolean {
		return this.status === 'loading' || this.status === 'idle';
	}

	/** Loads the value. Shows the loading state only when nothing is displayed yet. */
	async load(): Promise<void> {
		const generation = ++this.#generation;
		if (this.data === null) this.status = 'loading';
		else this.refreshing = true;
		try {
			const value = await this.#fetcher();
			if (generation !== this.#generation) return;
			this.data = value;
			this.error = null;
			this.status = 'ready';
		} catch (e) {
			if (generation !== this.#generation) return;
			this.error = toAppError(e);
			if (this.data === null) this.status = 'error';
		} finally {
			if (generation === this.#generation) this.refreshing = false;
		}
	}

	/**
	 * Loads only if nothing was ever requested: views sharing one costly value (the doctor
	 * report) reuse the result or the load in flight instead of starting their own.
	 */
	async ensure(): Promise<void> {
		if (this.status === 'idle') await this.load();
	}

	/** Applies an incremental update (from an event) on top of the current data. */
	patch(update: (current: T) => T): void {
		if (this.data === null) {
			// Nothing to patch yet: the in-flight (or a new) full load will carry the change.
			void this.load();
			return;
		}
		const loadInFlight = this.refreshing;
		this.#generation++;
		this.data = update(this.data);
		// The discarded in-flight response may carry other changes: fetch again.
		if (loadInFlight) void this.load();
		else this.refreshing = false;
	}

	/** Replaces the value with an authoritative one (e.g. a command result). */
	set(value: T): void {
		this.#generation++;
		this.data = value;
		this.error = null;
		this.status = 'ready';
	}
}
