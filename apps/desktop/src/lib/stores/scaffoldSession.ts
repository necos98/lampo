import type { AppError, ScaffoldProjectRequest, TaskOutcome } from '../api/index.js';

/** Setup of a new folder: filling the form, running (live log), failed (log + retry), done. */
export type ScaffoldPhase = 'form' | 'running' | 'failed' | 'done';

/** What the create screen shows for a setup task in a given state. */
export interface ScaffoldView {
	phase: ScaffoldPhase;
	/** The run was cancelled by the user (the form says so). */
	cancelled: boolean;
	error: AppError | null;
}

/** The view of a setup task: running while it has no outcome, then the outcome decides. */
export function scaffoldViewOf(outcome: TaskOutcome | null): ScaffoldView {
	if (!outcome) return { phase: 'running', cancelled: false, error: null };
	switch (outcome.status) {
		case 'succeeded':
			return { phase: 'done', cancelled: false, error: null };
		case 'cancelled':
			return { phase: 'form', cancelled: true, error: null };
		case 'failed':
			return { phase: 'failed', cancelled: false, error: outcome.error };
	}
}

/** The part of a finished task the session looks at. */
export interface FinishedTask {
	taskId: string;
	outcome: TaskOutcome | null;
}

/** Where the session hears about finished tasks (the tasks store). */
export interface TaskEvents {
	onFinished(listener: (task: FinishedTask) => void): () => void;
}

interface Pending {
	request: ScaffoldProjectRequest;
	taskId: string;
}

/**
 * The new-project setup the user started and has not registered yet. It outlives the create
 * screen: leaving the page while a clone or the Laravel Installer runs must not lose the folder.
 * The screen resumes from `request` when it comes back; while it is away, a successful end is
 * announced through `onReadyElsewhere` (a toast that leads back to the confirm step).
 */
export class ScaffoldSession {
	#pending: Pending | null = null;
	/** Create screens on display: they handle the end of the task themselves. */
	#viewers = 0;

	readonly #tasks: TaskEvents;
	readonly #onReadyElsewhere: () => void;

	constructor(tasks: TaskEvents, onReadyElsewhere: () => void) {
		this.#tasks = tasks;
		this.#onReadyElsewhere = onReadyElsewhere;
	}

	/** The request of the setup to resume, `null` when there is none. */
	get request(): ScaffoldProjectRequest | null {
		return this.#pending?.request ?? null;
	}

	/** A setup task was started (it replaces any earlier one). */
	began(request: ScaffoldProjectRequest, taskId: string): void {
		this.#pending = { request, taskId };
	}

	/** The folder became a project: nothing left to resume. */
	registered(): void {
		this.#pending = null;
	}

	/** The create screen is on display until the returned function is called. */
	attach(): () => void {
		this.#viewers++;
		let attached = true;
		return () => {
			if (!attached) return;
			attached = false;
			this.#viewers--;
		};
	}

	async start(): Promise<() => void> {
		return this.#tasks.onFinished((task) => this.#finished(task));
	}

	#finished(task: FinishedTask): void {
		if (task.taskId !== this.#pending?.taskId || this.#viewers > 0) return;
		if (task.outcome?.status === 'succeeded') this.#onReadyElsewhere();
	}
}
