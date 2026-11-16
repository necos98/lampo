import assert from 'node:assert/strict';
import { test } from 'node:test';

import { InFlight } from './inFlight.ts';

/** An operation that stays pending until `finish` is called. */
function deferred<T>() {
	let finish!: (value: T) => void;
	let fail!: (reason: unknown) => void;
	const promise = new Promise<T>((resolve, reject) => {
		finish = resolve;
		fail = reject;
	});
	return { promise, finish, fail };
}

test('a second run of the same key while the first is pending is refused without calling op', async () => {
	const inFlight = new InFlight();
	const first = deferred<string>();
	let calls = 0;
	const op = () => {
		calls++;
		return first.promise;
	};

	const a = inFlight.run('install:php|8.4|', op);
	const b = inFlight.run('install:php|8.4|', op);

	assert.equal(inFlight.has('install:php|8.4|'), true, 'held synchronously, before op resolves');
	assert.equal(await b, null);
	first.finish('task-1');
	assert.equal(await a, 'task-1');
	assert.equal(calls, 1);
	assert.equal(inFlight.has('install:php|8.4|'), false);
});

test('different keys run concurrently', async () => {
	const inFlight = new InFlight();
	const results = await Promise.all([inFlight.run('a', async () => 1), inFlight.run('b', async () => 2)]);
	assert.deepEqual(results, [1, 2]);
});

test('the key is released after op rejects, so the user can retry', async () => {
	const inFlight = new InFlight();
	const failing = deferred<never>();
	const run = inFlight.run('k', () => failing.promise);
	failing.fail(new Error('network'));

	await assert.rejects(run, /network/);
	assert.equal(inFlight.has('k'), false);
	assert.equal(await inFlight.run('k', async () => 'retried'), 'retried');
});

test('keys are kept in the record it was given', async () => {
	const held: Record<string, true> = {};
	const inFlight = new InFlight(held);
	const pending = deferred<void>();
	const run = inFlight.run('uninstall:php|8.3.12|', () => pending.promise);

	assert.deepEqual(Object.keys(held), ['uninstall:php|8.3.12|']);
	pending.finish();
	await run;
	assert.deepEqual(Object.keys(held), []);
});
