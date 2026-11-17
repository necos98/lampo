import { afterEach, describe, expect, it } from 'vitest';

import type { DatabaseMode } from '../api/index.js';
import { testBackend, testRunner } from '../testing/harness.js';
import { TasksStore } from './tasks.svelte.js';
import { WorktreesStore, worktreeTaskKey } from './worktrees.svelte.js';

const SHOP = 'shop';
const SHARED: DatabaseMode = { mode: 'shared' };
const FEATURE = 'C:\\Dev\\shop-feature';
const FIX = 'C:\\Dev\\shop-fix';

let stops: Array<() => void> = [];

afterEach(() => {
	stops.forEach((stop) => stop());
	stops = [];
});

/** A store whose creations get the task ids `task-1`, `task-2`… finished by the test. */
async function startStore() {
	let next = 0;
	const env = testBackend({ listRunningTasks: async () => [], createWorktree: async () => ({ taskId: `task-${++next}` }) });
	const tasks = new TasksStore(env.backend);
	stops.push(await tasks.start());
	return { store: new WorktreesStore(env.backend, testRunner(), tasks), tasks, env };
}

const paths = (store: WorktreesStore, projectId = SHOP) => store.creations(projectId).map((c) => c.path);

describe('WorktreesStore creations', () => {
	it('lists the creations started from the project, running or not yet succeeded', async () => {
		const { store, env } = await startStore();
		await store.create(SHOP, FEATURE, SHARED);
		await store.create(SHOP, FIX, SHARED);

		expect(paths(store)).toEqual([FEATURE, FIX]);
		expect(paths(store, 'blog')).toEqual([]);

		env.finish('task-1', { status: 'failed', error: { code: 'process-failed', message: 'git failed' } });
		env.finish('task-2', { status: 'succeeded' });

		expect(store.creations(SHOP)).toMatchObject([{ path: FEATURE, task: { taskId: 'task-1', outcome: { status: 'failed' } } }]);
	});

	it('follows nothing when the creation did not start', async () => {
		const env = testBackend({
			createWorktree: async () => {
				throw { code: 'validation', message: 'not a git repository' };
			}
		});
		const tasks = new TasksStore(env.backend);
		const store = new WorktreesStore(env.backend, testRunner(), tasks);

		await store.create(SHOP, FEATURE, SHARED);

		expect(paths(store)).toEqual([]);
	});

	it('keeps a single row when the same folder is started again', async () => {
		const { store, env } = await startStore();
		await store.create(SHOP, FEATURE, SHARED);
		env.finish('task-1', { status: 'cancelled' });
		await store.create(SHOP, FEATURE, SHARED);

		expect(store.creations(SHOP)).toMatchObject([{ path: FEATURE, task: { taskId: 'task-2', outcome: null } }]);
	});
});

describe('WorktreesStore.dismissCreation', () => {
	it('keeps a running creation: it is cancelled, not dismissed', async () => {
		const { store, tasks } = await startStore();
		await store.create(SHOP, FEATURE, SHARED);

		store.dismissCreation(SHOP, FEATURE);

		expect(paths(store)).toEqual([FEATURE]);
		expect(tasks.forKey(worktreeTaskKey(FEATURE))?.taskId).toBe('task-1');
	});

	it('closes a finished creation and forgets its task', async () => {
		const { store, tasks, env } = await startStore();
		await store.create(SHOP, FEATURE, SHARED);
		await store.create(SHOP, FIX, SHARED);
		env.finish('task-1', { status: 'failed', error: { code: 'process-failed', message: 'git failed' } });

		store.dismissCreation(SHOP, FEATURE);

		expect(paths(store)).toEqual([FIX]);
		expect(tasks.forKey(worktreeTaskKey(FEATURE))).toBeNull();
	});

	it('stops following a folder whose task is already gone', async () => {
		const { store, tasks, env } = await startStore();
		await store.create(SHOP, FEATURE, SHARED);
		env.finish('task-1', { status: 'cancelled' });
		tasks.dismiss('task-1');

		store.dismissCreation(SHOP, FEATURE);
		// The same folder created from elsewhere (the CLI) is no row of this panel any more.
		tasks.track('task-cli', { kind: 'worktree', key: worktreeTaskKey(FEATURE) });

		expect(paths(store)).toEqual([]);
	});
});
