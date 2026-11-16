/**
 * Finished tasks kept besides the running and the retained ones. A finished task still shows its
 * outcome (a failure on a package row, the log of a setup), so the most recent ones stay; older
 * ones go, otherwise every task of the session would stay in memory with its log.
 */
export const MAX_FINISHED_TASKS = 50;

interface Entry {
	/** Creation order: the latest task of a key is the last one created, as the store lists them. */
	order: number;
	key: string | null;
}

/**
 * The bookkeeping of the tasks store: the tasks of each key, so a lookup by key does not scan
 * every task, and which finished tasks to forget. A task can be retained (e.g. the new-project
 * setup the create screen resumes) and is then never forgotten until released.
 */
export class TaskIndex {
	/** Ids of the tasks of each key, oldest first. Pass a `SvelteMap` so `latest` is reactive in the UI. */
	readonly #byKey: Map<string, readonly string[]>;
	readonly #entries = new Map<string, Entry>();
	/** Finished tasks, the earliest finished first. */
	readonly #finished = new Set<string>();
	/** How many holders retain each task. */
	readonly #holders = new Map<string, number>();
	#created = 0;

	constructor(byKey: Map<string, readonly string[]> = new Map()) {
		this.#byKey = byKey;
	}

	/** A task the store started to follow; a task already known keeps its place. */
	add(taskId: string): void {
		if (!this.#entries.has(taskId)) this.#entries.set(taskId, { order: this.#created++, key: null });
	}

	/** Files a task under the key it was given. */
	setKey(taskId: string, key: string): void {
		this.add(taskId);
		const entry = this.#entry(taskId);
		if (entry.key === key) return;
		this.#unlink(taskId, entry);
		entry.key = key;
		const ids = this.#byKey.get(key) ?? [];
		const at = ids.findLastIndex((id) => this.#entry(id).order < entry.order) + 1;
		this.#byKey.set(key, [...ids.slice(0, at), taskId, ...ids.slice(at)]);
	}

	/** The last created task with this key, `null` when there is none. */
	latest(key: string): string | null {
		return this.#byKey.get(key)?.at(-1) ?? null;
	}

	/** The task ended; a second end of the same task changes nothing. */
	finish(taskId: string): void {
		if (this.#entries.has(taskId)) this.#finished.add(taskId);
	}

	remove(taskId: string): void {
		const entry = this.#entries.get(taskId);
		if (!entry) return;
		this.#unlink(taskId, entry);
		this.#entries.delete(taskId);
		this.#finished.delete(taskId);
	}

	/** Keeps the task from being forgotten until the returned function is called (once is enough). */
	retain(taskId: string): () => void {
		this.#holders.set(taskId, (this.#holders.get(taskId) ?? 0) + 1);
		let held = true;
		return () => {
			if (!held) return;
			held = false;
			const left = (this.#holders.get(taskId) ?? 1) - 1;
			if (left > 0) this.#holders.set(taskId, left);
			else this.#holders.delete(taskId);
		};
	}

	/**
	 * Forgets the finished tasks that are not retained, except the `limit` most recently finished.
	 * Returns the ids it forgot, for the store to drop their state.
	 */
	prune(limit = MAX_FINISHED_TASKS): string[] {
		const candidates = [...this.#finished].filter((id) => !this.#holders.has(id));
		const forgotten = candidates.slice(0, Math.max(0, candidates.length - limit));
		for (const id of forgotten) this.remove(id);
		return forgotten;
	}

	#entry(taskId: string): Entry {
		const entry = this.#entries.get(taskId);
		if (!entry) throw new Error(`unknown task ${taskId}`);
		return entry;
	}

	#unlink(taskId: string, entry: Entry): void {
		if (entry.key === null) return;
		const ids = (this.#byKey.get(entry.key) ?? []).filter((id) => id !== taskId);
		if (ids.length > 0) this.#byKey.set(entry.key, ids);
		else this.#byKey.delete(entry.key);
	}
}
