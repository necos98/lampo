import type { DatabaseMode, PruneReport, WorktreeEnvironment } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner } from './runner.js';
import type { TasksStore } from './tasks.svelte.js';

export const worktreeTaskKey = (path: string) => `worktree:${path}`;

export class WorktreesStore {
	readonly list: Resource<WorktreeEnvironment[]>;
	removing = $state<Record<string, boolean>>({});

	readonly #backend: Backend;
	readonly #runner: CommandRunner;
	readonly #tasks: TasksStore;

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
		if (result.ok) this.#tasks.track(result.value.taskId, { kind: 'worktree', key: worktreeTaskKey(path) });
		return result;
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
		const result = await this.#runner.run<PruneReport>(() => this.#backend.commands.pruneWorktrees({ projectId, dryRun: false, dropDatabases }), { success: m.worktrees.pruned });
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
