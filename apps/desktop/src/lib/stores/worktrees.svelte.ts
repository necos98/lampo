import type { DatabaseMode, PruneReport, WorktreeEnvironment } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { FollowedWorktrees } from './followedWorktrees.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner } from './runner.js';
import type { TaskState, TasksStore } from './tasks.svelte.js';

export const worktreeTaskKey = (path: string) => `worktree:${path}`;

/** A worktree creation started from a project's panel that has not succeeded (yet). */
export interface WorktreeCreation {
	path: string;
	task: TaskState;
}

export class WorktreesStore {
	readonly list: Resource<WorktreeEnvironment[]>;
	removing = $state<Record<string, boolean>>({});

	readonly #backend: Backend;
	readonly #runner: CommandRunner;
	readonly #tasks: TasksStore;
	/** Kept here and not in the panel, so the rows survive the panel being unmounted. */
	#followedPaths = $state<Record<string, string[]>>({});
	readonly #followed = new FollowedWorktrees(this.#followedPaths);

	constructor(backend: Backend, runner: CommandRunner, tasks: TasksStore) {
		this.#backend = backend;
		this.#runner = runner;
		this.#tasks = tasks;
		this.list = new Resource(() => backend.commands.listWorktrees(null));
	}

	load = () => this.list.load();

	forProject(projectId: string): WorktreeEnvironment[] {
		return (this.list.data ?? []).filter((w) => w.projectId === projectId);
	}

	async create(projectId: string, path: string, database: DatabaseMode) {
		const result = await this.#runner.run(() => this.#backend.commands.createWorktree({ path, projectId, database }));
		if (result.ok) {
			this.#tasks.track(result.value.taskId, { kind: 'worktree', key: worktreeTaskKey(path) });
			this.#followed.follow(projectId, path);
		}
		return result;
	}

	/** Creations of this project that are running, failed or cancelled; a succeeded one shows up in the list instead. */
	creations(projectId: string): WorktreeCreation[] {
		return this.#followed
			.forProject(projectId)
			.map((path) => ({ path, task: this.#tasks.forKey(worktreeTaskKey(path)) }))
			.filter((c): c is WorktreeCreation => c.task !== null && c.task.outcome?.status !== 'succeeded');
	}

	/** Closes the row of a finished creation; a running one stays (it is cancelled, not dismissed). */
	dismissCreation(projectId: string, path: string): void {
		const task = this.#tasks.forKey(worktreeTaskKey(path));
		if (task && !task.outcome) return;
		if (task) this.#tasks.dismiss(task.taskId);
		this.#followed.unfollow(projectId, path);
	}

	async remove(site: string, dropDatabase: boolean) {
		this.removing[site] = true;
		const result = await this.#runner.run(() => this.#backend.commands.removeWorktree({ target: site, dropDatabase }), { success: m.worktrees.removed });
		delete this.removing[site];
		if (result.ok) await this.list.load();
		return result;
	}

	/** Dry run first so the user sees what would go; `confirm` performs the removal. */
	preview(projectId: string) {
		return this.#runner.run(() => this.#backend.commands.pruneWorktrees({ projectId, dryRun: true }));
	}

	async prune(projectId: string, dropDatabases: boolean) {
		const result = await this.#runner.run<PruneReport>(() => this.#backend.commands.pruneWorktrees({ projectId, dryRun: false, dropDatabases }), {
			success: m.worktrees.pruned,
			warning: (r) => (r.failed.length ? m.worktrees.prunedPartially(r.failed.map((f) => `${f.site}: ${f.message}`).join('; ')) : null)
		});
		if (result.ok) await this.list.load();
		return result;
	}

	async start(): Promise<() => void> {
		const stop = await this.#backend.events.onWorktreesChanged(() => void this.list.load());
		const stopTask = this.#tasks.onFinished((t) => {
			if (t.meta?.kind === 'worktree') void this.list.load();
		});
		return () => {
			stop();
			stopTask();
		};
	}
}
