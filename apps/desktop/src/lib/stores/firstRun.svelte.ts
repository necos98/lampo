import type { FirstRunItem, FirstRunStatus, TaskProgressPayload } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner } from './runner.js';
import type { TasksStore } from './tasks.svelte.js';

export const FIRST_RUN_TASK = 'first-run';

export type ItemPhase = 'done' | 'active' | 'pending' | 'failed';

/** State of the first-run download set. */
export class FirstRunStore {
	readonly status: Resource<FirstRunStatus>;
	starting = $state(false);

	readonly #backend: Backend;
	readonly #runner: CommandRunner;
	readonly #tasks: TasksStore;

	constructor(backend: Backend, runner: CommandRunner, tasks: TasksStore) {
		this.#backend = backend;
		this.#runner = runner;
		this.#tasks = tasks;
		this.status = new Resource(() => backend.commands.firstRunStatus());
	}

	load = () => this.status.load();

	get needsSetup(): boolean {
		return this.status.data !== null && !this.status.data.completed;
	}

	get task() {
		return this.#tasks.forKey(FIRST_RUN_TASK);
	}

	get running(): boolean {
		return this.#tasks.isRunning(FIRST_RUN_TASK);
	}

	/** Latest progress of the package that belongs to `item`. The install reports every stage
	 * under the package kind; the exact key wins over older, more specific ones. */
	progressOf(item: FirstRunItem): TaskProgressPayload | null {
		const subjects = this.task?.subjects ?? {};
		if (subjects[item.kind]) return subjects[item.kind];
		const hits = Object.entries(subjects).filter(([subject]) => subject.startsWith(`${item.kind}-`) || subject.startsWith(`${item.kind} `));
		return hits.length ? hits[hits.length - 1][1] : null;
	}

	phaseOf(item: FirstRunItem): ItemPhase {
		if (item.installed) return 'done';
		const task = this.task;
		if (!task) return 'pending';
		const progress = this.progressOf(item);
		// The last stage of an install: done before the status is reloaded at the very end.
		if (progress?.stage === 'configuring' && progress.total != null && progress.done >= progress.total) return 'done';
		// On failure only the package being processed when it happened is flagged.
		if (task.outcome?.status === 'failed') return progress && progress.subject === task.progress?.subject ? 'failed' : 'pending';
		return progress ? 'active' : 'pending';
	}

	async run() {
		this.starting = true;
		if (this.task) this.#tasks.dismiss(this.task.taskId);
		const result = await this.#runner.run(() => this.#backend.commands.runFirstRun());
		this.starting = false;
		if (result.ok) this.#tasks.track(result.value.taskId, { kind: 'first-run', key: FIRST_RUN_TASK });
		return result;
	}

	async start(): Promise<() => void> {
		return this.#tasks.onFinished((t) => {
			if (t.meta?.kind === 'first-run') void this.status.load();
		});
	}
}
