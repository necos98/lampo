import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { AppError, ScaffoldProjectRequest, TaskOutcome } from '../api/index.js';
import { ScaffoldSession, scaffoldViewOf, type FinishedTask, type TaskEvents } from './scaffoldSession.ts';

/** A tasks store reduced to its finished events, fired by hand. */
function fakeTasks() {
	const listeners = new Set<(task: FinishedTask) => void>();
	const events: TaskEvents = {
		onFinished(listener) {
			listeners.add(listener);
			return () => void listeners.delete(listener);
		}
	};
	const finish = (taskId: string, outcome: TaskOutcome) => listeners.forEach((l) => l({ taskId, outcome }));
	return { events, finish, listeners };
}

const CLONE: ScaffoldProjectRequest = { parentDir: 'C:\\Dev', folderName: 'shop', source: { kind: 'git-clone', url: 'https://example.test/shop.git', branch: null } };
const SUCCEEDED: TaskOutcome = { status: 'succeeded' };

async function startedSession() {
	const tasks = fakeTasks();
	let announced = 0;
	const session = new ScaffoldSession(tasks.events, () => announced++);
	const stop = await session.start();
	return { session, tasks, stop, announced: () => announced };
}

test('the request of a started setup is still there after the create screen is left', async () => {
	const { session } = await startedSession();
	const detach = session.attach();
	session.began(CLONE, 'task-1');
	detach();

	assert.deepEqual(session.request, CLONE, 'a later visit can resume from it');
});

test('a setup that ends while the user is elsewhere is announced once', async () => {
	const { session, tasks, announced } = await startedSession();
	session.began(CLONE, 'task-1');

	tasks.finish('task-1', SUCCEEDED);

	assert.equal(announced(), 1);
	assert.deepEqual(session.request, CLONE, 'the folder still waits to be registered');
});

test('a setup that ends while the create screen is on display is left to the screen', async () => {
	const { session, tasks, announced } = await startedSession();
	session.attach();
	session.began(CLONE, 'task-1');

	tasks.finish('task-1', SUCCEEDED);

	assert.equal(announced(), 0);
});

test('a failed, cancelled or unrelated task is not announced as ready', async () => {
	const { session, tasks, announced } = await startedSession();
	session.began(CLONE, 'task-1');
	const error: AppError = { code: 'process-failed', message: 'git clone failed', details: { output: 'fatal: not found' } };

	tasks.finish('task-1', { status: 'failed', error });
	tasks.finish('task-1', { status: 'cancelled' });
	tasks.finish('task-2', SUCCEEDED);

	assert.equal(announced(), 0);
});

test('detaching twice does not hide a later screen', async () => {
	const { session, tasks, announced } = await startedSession();
	const detachFirst = session.attach();
	detachFirst();
	detachFirst();
	session.attach();
	session.began(CLONE, 'task-1');

	tasks.finish('task-1', SUCCEEDED);

	assert.equal(announced(), 0, 'the second screen is still on display');
});

test('once registered there is nothing to resume or announce', async () => {
	const { session, tasks, announced } = await startedSession();
	session.began(CLONE, 'task-1');

	session.registered();
	tasks.finish('task-1', SUCCEEDED);

	assert.equal(session.request, null);
	assert.equal(announced(), 0);
});

test('a newer setup replaces the earlier one', async () => {
	const { session, tasks, announced } = await startedSession();
	session.began(CLONE, 'task-1');
	const laravel: ScaffoldProjectRequest = { parentDir: 'C:\\Dev', folderName: 'blog', source: { kind: 'laravel', phpVersion: '8.4' } };
	session.began(laravel, 'task-2');

	tasks.finish('task-1', SUCCEEDED);

	assert.deepEqual(session.request, laravel);
	assert.equal(announced(), 0, 'the replaced task is no longer followed');
});

test('stopping the session removes its listener', async () => {
	const { tasks, stop } = await startedSession();

	stop();

	assert.equal(tasks.listeners.size, 0);
});

test('the step shown for a resumed task follows its outcome', () => {
	const error: AppError = { code: 'process-failed', message: 'git clone failed', details: { output: 'fatal: not found' } };

	assert.deepEqual(scaffoldViewOf(null), { phase: 'running', cancelled: false, error: null });
	assert.deepEqual(scaffoldViewOf(SUCCEEDED), { phase: 'done', cancelled: false, error: null });
	assert.deepEqual(scaffoldViewOf({ status: 'cancelled' }), { phase: 'form', cancelled: true, error: null });
	assert.deepEqual(scaffoldViewOf({ status: 'failed', error }), { phase: 'failed', cancelled: false, error });
});
