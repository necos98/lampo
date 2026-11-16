import { SvelteMap } from 'svelte/reactivity';

import type { AppError, RunningTask, ScaffoldProjectRequest, TaskOperation, TaskOutcome, TaskProgressPayload } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { installTaskKey, uninstallTaskKey } from '../packages/grouping.js';
import { FIRST_RUN_TASK } from './firstRun.svelte.js';
import { TaskIndex } from './taskIndex.js';
import { worktreeTaskKey } from './worktrees.svelte.js';

/** Output lines kept per task (the log of a running new-project setup). */
export const MAX_TASK_LINES = 200;

/** Key of a new-project setup task: the folder it creates. */
export function scaffoldTaskKey(request: ScaffoldProjectRequest): string {
	return `scaffold:${request.parentDir.replace(/[\\/]+$/, '')}\\${request.folderName ?? ''}`.toLowerCase();
}

/** The meaning the store that started a task gave it, rebuilt from what the backend recorded. */
export function metaOf(operation: TaskOperation): TaskMeta {
	switch (operation.kind) {
		case 'install-package': {
			const r = operation.request;
			return { kind: 'install', key: installTaskKey(r.kind, r.series ?? '', r.phpVersion ?? null) };
		}
		case 'first-run':
			return { kind: 'first-run', key: FIRST_RUN_TASK };
		case 'worktree-up':
			return { kind: 'worktree', key: worktreeTaskKey(operation.request.path) };
		case 'scaffold-project':
			return { kind: 'scaffold', key: scaffoldTaskKey(operation.request) };
		case 'uninstall-package':
			return { kind: 'uninstall', key: uninstallTaskKey(operation.id) };
	}
}

export interface TaskMeta {
	kind: 'install' | 'uninstall' | 'first-run' | 'worktree' | 'scaffold';
	/** What the task is about; lookups use it (`php:8.4`, `first-run`, `worktree:shop@x`). */
	key: string;
}

export interface TaskState {
	taskId: string;
	meta: TaskMeta | null;
	/** Latest progress event. */
	progress: TaskProgressPayload | null;
	/** Latest progress event per subject (a first run downloads several packages). */
	subjects: Record<string, TaskProgressPayload>;
	outcome: TaskOutcome | null;
	/** Output lines of the programs the task runs (`running` progress messages), the last `MAX_TASK_LINES`. */
	lines: string[];
}

type FinishedListener = (task: TaskState) => void;

/**
 * Background tasks. Entries are created by whichever comes first, the event or `track`, because
 * a progress or finished event can arrive before the command that started the task has resolved.
 * Finished tasks are forgotten beyond the most recent `MAX_FINISHED_TASKS` (`taskIndex.ts`), unless retained.
 */
export class TasksStore {
	items = $state<Record<string, TaskState>>({});

	readonly #backend: Backend;
	readonly #index = new TaskIndex(new SvelteMap());
	readonly #listeners = new Set<FinishedListener>();

	constructor(backend: Backend) {
		this.#backend = backend;
	}

	#ensure(taskId: string): TaskState {
		if (!this.items[taskId]) {
			this.items[taskId] = { taskId, meta: null, progress: null, subjects: {}, outcome: null, lines: [] };
			this.#index.add(taskId);
		}
		return this.items[taskId];
	}

	/** Attaches meaning to a task id returned by a command. */
	track(taskId: string, meta: TaskMeta): void {
		const task = this.#ensure(taskId);
		task.meta = meta;
		this.#index.setKey(taskId, meta.key);
		if (task.outcome) this.#notify(task);
	}

	/** The latest task with this key, running or finished. */
	forKey(key: string): TaskState | null {
		const taskId = this.#index.latest(key);
		return taskId === null ? null : (this.items[taskId] ?? null);
	}

	isRunning(key: string): boolean {
		const t = this.forKey(key);
		return t !== null && t.outcome === null;
	}

	failure(key: string): AppError | null {
		const o = this.forKey(key)?.outcome;
		return o?.status === 'failed' ? o.error : null;
	}

	/** Forget a task (e.g. after the user dismissed its failure). */
	dismiss(taskId: string): void {
		delete this.items[taskId];
		this.#index.remove(taskId);
	}

	/** Keeps a task, finished or not, until the returned function is called (a flow that resumes it). */
	retain(taskId: string): () => void {
		return this.#index.retain(taskId);
	}

	async cancel(taskId: string): Promise<void> {
		try {
			await this.#backend.commands.cancelTask(taskId);
		} catch {
			// Cancelling a task that already ended is not an error for the user.
		}
	}

	/** Called once per finished tracked task. Returns an unsubscribe function. */
	onFinished(listener: FinishedListener): () => void {
		this.#listeners.add(listener);
		return () => void this.#listeners.delete(listener);
	}

	#notify(task: TaskState): void {
		for (const l of this.#listeners) l(task);
	}

	#forgetOldFinished(): void {
		for (const taskId of this.#index.prune()) delete this.items[taskId];
	}

	/** Adopts tasks that were already running when this window was created (state restored). */
	#adopt(running: RunningTask[]): void {
		for (const r of running) {
			const task = this.#ensure(r.taskId);
			if (task.outcome) continue;
			if (!task.meta) {
				task.meta = metaOf(r.operation);
				this.#index.setKey(r.taskId, task.meta.key);
			}
			if (!task.progress && r.progress) task.progress = { taskId: r.taskId, ...r.progress };
			for (const s of r.subjects) if (s.subject && !task.subjects[s.subject]) task.subjects[s.subject] = { taskId: r.taskId, ...s };
		}
	}

	async start(): Promise<() => void> {
		const { events } = this.#backend;
		const stops = await Promise.all([
			events.onTaskProgress((p) => {
				const task = this.#ensure(p.taskId);
				if (task.outcome) return; // a late progress event after the end changes nothing
				task.progress = p;
				if (p.subject) task.subjects[p.subject] = p;
				if (p.message) {
					task.lines.push(p.message);
					if (task.lines.length > MAX_TASK_LINES) task.lines.splice(0, task.lines.length - MAX_TASK_LINES);
				}
			}),
			events.onTaskFinished((p) => {
				const task = this.#ensure(p.taskId);
				task.outcome = p.outcome;
				this.#index.finish(p.taskId);
				if (task.meta) this.#notify(task);
				this.#forgetOldFinished();
			})
		]);
		// Subscribed first, so a task finishing meanwhile is not shown as running forever.
		try {
			this.#adopt(await this.#backend.commands.listRunningTasks());
		} catch {
			// Not fatal: new tasks are still followed through events.
		}
		return () => stops.forEach((s) => s());
	}
}
