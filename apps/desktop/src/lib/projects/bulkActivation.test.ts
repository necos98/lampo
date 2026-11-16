import assert from 'node:assert/strict';
import { test } from 'node:test';

import { bulkActivationTargets, setEachActive } from './bulkActivation.ts';

const projects = [
	{ id: 'shop', active: true, folderExists: true },
	{ id: 'blog', active: false, folderExists: true },
	{ id: 'api', active: false, folderExists: true },
	{ id: 'old', active: false, folderExists: false },
	{ id: 'gone', active: true, folderExists: false }
];

test('targets are the projects not already in the requested state, never a missing folder', () => {
	assert.deepEqual(bulkActivationTargets(projects, true), ['blog', 'api']);
	assert.deepEqual(bulkActivationTargets(projects, false), ['shop']);
	assert.deepEqual(bulkActivationTargets([], true), []);
});

test('every id is applied in order, one at a time', async () => {
	const calls: string[] = [];
	let running = 0;
	const result = await setEachActive(['a', 'b', 'c'], async (id) => {
		running++;
		assert.equal(running, 1, 'changes must not overlap');
		calls.push(id);
		await Promise.resolve();
		running--;
	});
	assert.deepEqual(calls, ['a', 'b', 'c']);
	assert.deepEqual(result, { changed: ['a', 'b', 'c'], failed: [], elevationPending: false });
});

test('a refusal is collected and the rest still runs', async () => {
	const refusal = { code: 'port-in-use', message: 'busy' };
	const result = await setEachActive(['a', 'b', 'c'], async (id) => {
		if (id === 'b') throw refusal;
	});
	assert.deepEqual(result.changed, ['a', 'c']);
	assert.deepEqual(result.failed, [{ id: 'b', error: refusal }]);
	assert.equal(result.elevationPending, false);
});

test('a change waiting for the hosts file counts as saved and flags the elevation', async () => {
	const result = await setEachActive(['a', 'b'], async (id) => {
		if (id === 'a') throw { code: 'elevation-required', message: 'hosts' };
	});
	assert.deepEqual(result.changed, ['a', 'b']);
	assert.deepEqual(result.failed, []);
	assert.equal(result.elevationPending, true);
});

test('non-object rejections are failures, not elevation', async () => {
	const result = await setEachActive(['a', 'b'], async (id) => {
		throw id === 'a' ? 'elevation-required' : null;
	});
	assert.deepEqual(result.failed.map((f) => f.id), ['a', 'b']);
	assert.equal(result.elevationPending, false);
});

test('no ids, no calls', async () => {
	let called = false;
	const result = await setEachActive([], async () => {
		called = true;
	});
	assert.equal(called, false);
	assert.deepEqual(result, { changed: [], failed: [], elevationPending: false });
});
