import assert from 'node:assert/strict';
import { test } from 'node:test';

import { MAX_FINISHED_TASKS, TaskIndex } from './taskIndex.ts';

/** Adds `count` finished tasks `<prefix>-0`, `<prefix>-1`, … in this order. */
function finishMany(index: TaskIndex, prefix: string, count: number): string[] {
	const ids = Array.from({ length: count }, (_, i) => `${prefix}-${i}`);
	for (const id of ids) {
		index.add(id);
		index.finish(id);
	}
	return ids;
}

test('a key resolves to its last created task', () => {
	const index = new TaskIndex();
	index.setKey('task-1', 'php:8.4');
	index.setKey('task-2', 'php:8.4');
	index.setKey('task-3', 'php:8.3');

	assert.equal(index.latest('php:8.4'), 'task-2');
	assert.equal(index.latest('php:8.3'), 'task-3');
	assert.equal(index.latest('first-run'), null);
});

test('creation order wins over the order the keys were given', () => {
	const index = new TaskIndex();
	index.add('task-1'); // its progress event arrived before the command resolved
	index.add('task-2');
	index.setKey('task-2', 'php:8.4');
	index.setKey('task-1', 'php:8.4');

	assert.equal(index.latest('php:8.4'), 'task-2');
});

test('a task given another key moves to it', () => {
	const index = new TaskIndex();
	index.setKey('task-1', 'php:8.4');

	index.setKey('task-1', 'php:8.3');

	assert.equal(index.latest('php:8.4'), null);
	assert.equal(index.latest('php:8.3'), 'task-1');
});

test('removing the latest task of a key falls back to the earlier one', () => {
	const byKey = new Map<string, readonly string[]>();
	const index = new TaskIndex(byKey);
	index.setKey('task-1', 'first-run');
	index.setKey('task-2', 'first-run');

	index.remove('task-2');
	assert.equal(index.latest('first-run'), 'task-1');

	index.remove('task-1');
	assert.equal(index.latest('first-run'), null);
	assert.equal(byKey.size, 0, 'an emptied key leaves nothing behind');
});

test('only the most recently finished tasks are kept', () => {
	const index = new TaskIndex();
	const ids = finishMany(index, 'task', MAX_FINISHED_TASKS + 2);
	index.setKey(ids[0], 'php:8.4');

	assert.deepEqual(index.prune(), ids.slice(0, 2));
	assert.equal(index.latest('php:8.4'), null, 'a forgotten task is no longer found by key');
	assert.deepEqual(index.prune(), [], 'nothing more to forget');
});

test('a task is as recent as its end, not its start', () => {
	const index = new TaskIndex();
	index.add('long-install');
	const quick = finishMany(index, 'quick', 2);
	index.finish('long-install');

	assert.deepEqual(index.prune(2), [quick[0]]);
});

test('running tasks are never forgotten', () => {
	const index = new TaskIndex();
	index.setKey('running', 'php:8.4');
	finishMany(index, 'done', 3);

	index.prune(0);

	assert.equal(index.latest('php:8.4'), 'running');
});

test('a retained task outlives the limit until released', () => {
	const index = new TaskIndex();
	index.setKey('setup', 'scaffold:c:\\dev\\shop');
	index.finish('setup');
	const release = index.retain('setup');
	finishMany(index, 'later', 2);

	assert.deepEqual(index.prune(1), ['later-0'], 'retained tasks do not count towards the limit');
	assert.equal(index.latest('scaffold:c:\\dev\\shop'), 'setup');

	release();
	assert.deepEqual(index.prune(1), ['setup']);
});

test('a task retained twice stays until both holders release it, releasing twice counts once', () => {
	const index = new TaskIndex();
	index.add('setup');
	index.finish('setup');
	const first = index.retain('setup');
	const second = index.retain('setup');

	first();
	first();
	assert.deepEqual(index.prune(0), []);

	second();
	assert.deepEqual(index.prune(0), ['setup']);
});

test('a task can be retained before the store knows it', () => {
	const index = new TaskIndex();
	index.retain('setup');
	index.setKey('setup', 'scaffold:c:\\dev\\shop');
	index.finish('setup');

	assert.deepEqual(index.prune(0), []);
});

test('ending twice or ending an unknown task changes nothing', () => {
	const index = new TaskIndex();
	index.add('task-1');
	index.add('task-2');
	index.finish('task-1');
	index.finish('task-2');
	index.finish('task-1');
	index.finish('unknown');

	assert.deepEqual(index.prune(1), ['task-1'], 'the first end decides the order');
});
