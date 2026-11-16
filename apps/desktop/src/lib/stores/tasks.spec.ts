import { afterEach, describe, expect, it } from 'vitest';

import type { TaskProgressPayload } from '../api/index.js';
import { testBackend, type TestBackend } from '../testing/harness.js';
import { MAX_FINISHED_TASKS } from './taskIndex.js';
import { MAX_TASK_LINES, TasksStore, type TaskMeta } from './tasks.svelte.js';

const INSTALL_PHP: TaskMeta = { kind: 'install', key: 'php:8.4' };
const SUCCEEDED = { status: 'succeeded' } as const;

const progress = (taskId: string, over: Partial<TaskProgressPayload> = {}): TaskProgressPayload => ({
	taskId,
	stage: 'downloading',
	done: 1,
	total: 10,
	subject: null,
	...over
});

let stops: Array<() => void> = [];

async function startStore(overrides: Parameters<typeof testBackend>[0] = {}): Promise<{ store: TasksStore; env: TestBackend }> {
	const env = testBackend({ listRunningTasks: async () => [], ...overrides });
	const store = new TasksStore(env.backend);
	stops.push(await store.start());
	return { store, env };
}

afterEach(() => {
	stops.forEach((stop) => stop());
	stops = [];
});

describe('TasksStore updates', () => {
	it('creates the task from a progress event that arrives before track()', async () => {
		const { store, env } = await startStore();

		env.progress(progress('t1'));
		store.track('t1', INSTALL_PHP);

		expect(store.items['t1'].progress?.done).toBe(1);
		expect(store.isRunning('php:8.4')).toBe(true);
	});

	it('keeps the latest progress per subject', async () => {
		const { store, env } = await startStore();
		store.track('t1', INSTALL_PHP);

		env.progress(progress('t1', { subject: 'php', done: 2 }));
		env.progress(progress('t1', { subject: 'composer', done: 3 }));
		env.progress(progress('t1', { subject: 'php', done: 5 }));

		const task = store.items['t1'];
		expect(task.progress?.subject).toBe('php');
		expect(task.subjects['php'].done).toBe(5);
		expect(task.subjects['composer'].done).toBe(3);
	});

	it('keeps only the last MAX_TASK_LINES output lines', async () => {
		const { store, env } = await startStore();
		store.track('t1', INSTALL_PHP);

		for (let i = 0; i < MAX_TASK_LINES + 25; i++) env.progress(progress('t1', { stage: 'running', message: `line ${i}` }));

		const { lines } = store.items['t1'];
		expect(lines).toHaveLength(MAX_TASK_LINES);
		expect(lines[0]).toBe('line 25');
		expect(lines.at(-1)).toBe(`line ${MAX_TASK_LINES + 24}`);
	});

	it('ignores a progress event that arrives after the outcome', async () => {
		const { store, env } = await startStore();
		store.track('t1', INSTALL_PHP);
		env.progress(progress('t1', { done: 4 }));

		env.finish('t1', SUCCEEDED);
		env.progress(progress('t1', { done: 9, message: 'late' }));

		expect(store.items['t1'].progress?.done).toBe(4);
		expect(store.items['t1'].lines).toEqual([]);
		expect(store.isRunning('php:8.4')).toBe(false);
	});

	it('exposes the error of a failed task through failure()', async () => {
		const { store, env } = await startStore();
		store.track('t1', INSTALL_PHP);
		const error = { code: 'network', message: 'offline' } as const;

		env.finish('t1', { status: 'failed', error });

		expect(store.failure('php:8.4')).toEqual(error);
	});

	it('notifies finished listeners once the task is tracked, whichever comes first', async () => {
		const { store, env } = await startStore();
		const finished: string[] = [];
		store.onFinished((t) => finished.push(t.taskId));

		store.track('early', INSTALL_PHP);
		env.finish('early', SUCCEEDED);
		env.finish('late', SUCCEEDED);
		store.track('late', { kind: 'install', key: 'php:8.3' });
		env.finish('untracked', SUCCEEDED);

		expect(finished).toEqual(['early', 'late']);
	});

	it('stops notifying after the listener unsubscribes', async () => {
		const { store, env } = await startStore();
		const finished: string[] = [];
		const stop = store.onFinished((t) => finished.push(t.taskId));
		stop();

		store.track('t1', INSTALL_PHP);
		env.finish('t1', SUCCEEDED);

		expect(finished).toEqual([]);
	});

	it('stops following events when the returned function is called', async () => {
		const { store, env } = await startStore();
		stops.pop()!();

		env.progress(progress('t1'));

		expect(store.items['t1']).toBeUndefined();
	});

	it('does not fail when cancelling a task that already ended', async () => {
		const { store } = await startStore({
			cancelTask: async () => {
				throw { code: 'not-found', message: 'gone' };
			}
		});

		await expect(store.cancel('t1')).resolves.toBeUndefined();
	});
});

describe('TasksStore pruning', () => {
	it('forgets the oldest finished tasks beyond the limit', async () => {
		const { store, env } = await startStore();
		const total = MAX_FINISHED_TASKS + 3;

		for (let i = 0; i < total; i++) {
			store.track(`t${i}`, { kind: 'install', key: `k${i}` });
			env.finish(`t${i}`, SUCCEEDED);
		}

		expect(Object.keys(store.items)).toHaveLength(MAX_FINISHED_TASKS);
		expect(store.items['t0']).toBeUndefined();
		expect(store.forKey('k0')).toBeNull();
		expect(store.forKey(`k${total - 1}`)).not.toBeNull();
	});

	it('never forgets a running task', async () => {
		const { store, env } = await startStore();
		store.track('running', INSTALL_PHP);

		for (let i = 0; i < MAX_FINISHED_TASKS + 5; i++) {
			store.track(`t${i}`, { kind: 'install', key: `k${i}` });
			env.finish(`t${i}`, SUCCEEDED);
		}

		expect(store.isRunning('php:8.4')).toBe(true);
	});

	it('keeps a retained task until it is released', async () => {
		const { store, env } = await startStore();
		store.track('kept', INSTALL_PHP);
		const release = store.retain('kept');
		env.finish('kept', SUCCEEDED);

		for (let i = 0; i < MAX_FINISHED_TASKS + 1; i++) {
			store.track(`t${i}`, { kind: 'install', key: `k${i}` });
			env.finish(`t${i}`, SUCCEEDED);
		}
		expect(store.forKey('php:8.4')).not.toBeNull();

		release();
		store.track('next', { kind: 'install', key: 'next' });
		env.finish('next', SUCCEEDED);
		expect(store.forKey('php:8.4')).toBeNull();
	});

	it('dismiss() forgets a task and its key', async () => {
		const { store, env } = await startStore();
		store.track('t1', INSTALL_PHP);
		env.finish('t1', { status: 'failed', error: { code: 'network', message: 'offline' } });

		store.dismiss('t1');

		expect(store.forKey('php:8.4')).toBeNull();
		expect(store.failure('php:8.4')).toBeNull();
	});
});

describe('TasksStore.forKey', () => {
	it('returns null for an unknown key', async () => {
		const { store } = await startStore();
		expect(store.forKey('nothing')).toBeNull();
		expect(store.isRunning('nothing')).toBe(false);
	});

	it('returns the latest task of a key, running or finished', async () => {
		const { store, env } = await startStore();
		store.track('old', INSTALL_PHP);
		env.finish('old', { status: 'failed', error: { code: 'network', message: 'offline' } });
		store.track('new', INSTALL_PHP);

		expect(store.forKey('php:8.4')?.taskId).toBe('new');
		expect(store.isRunning('php:8.4')).toBe(true);
		expect(store.failure('php:8.4')).toBeNull();
	});
});

describe('TasksStore adoption of running tasks', () => {
	it('adopts a task that was already running, with the meaning rebuilt from its operation', async () => {
		const { store } = await startStore({
			listRunningTasks: async () => [
				{
					taskId: 'restored',
					operation: { kind: 'first-run' },
					progress: { stage: 'downloading', done: 3, total: 9, subject: 'php' },
					subjects: [{ stage: 'downloading', done: 3, total: 9, subject: 'php' }]
				}
			]
		});

		const task = store.forKey('first-run');
		expect(task?.taskId).toBe('restored');
		expect(task?.progress?.done).toBe(3);
		expect(task?.subjects['php'].taskId).toBe('restored');
		expect(store.isRunning('first-run')).toBe(true);
	});

	it('does not adopt a task that finished while the list was being fetched', async () => {
		const env = testBackend({
			listRunningTasks: async () => {
				env.finish('restored', SUCCEEDED);
				return [{ taskId: 'restored', operation: { kind: 'first-run' }, progress: null, subjects: [] }];
			}
		});
		const store = new TasksStore(env.backend);
		stops.push(await store.start());

		expect(store.items['restored'].outcome).toEqual(SUCCEEDED);
		expect(store.items['restored'].meta).toBeNull();
	});

	it('still follows events when listing the running tasks fails', async () => {
		const { store, env } = await startStore({
			listRunningTasks: async () => {
				throw { code: 'internal', message: 'boom' };
			}
		});

		env.progress(progress('t1'));

		expect(store.items['t1'].progress?.done).toBe(1);
	});
});

