import assert from 'node:assert/strict';
import { afterEach, beforeEach, mock, test } from 'node:test';

import { Poller } from './poller.ts';

const INTERVAL = 1000;

beforeEach(() => mock.timers.enable({ apis: ['setTimeout'] }));
afterEach(() => mock.timers.reset());

/** Lets the awaited tick and its `finally` run. */
const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

function counting() {
	let calls = 0;
	const poller = new Poller(async () => {
		calls++;
	}, INTERVAL);
	return { poller, calls: () => calls };
}

test('ticks once per interval after start', async () => {
	const { poller, calls } = counting();
	poller.start();
	assert.equal(calls(), 0);
	mock.timers.tick(INTERVAL);
	await settle();
	assert.equal(calls(), 1);
	mock.timers.tick(INTERVAL);
	await settle();
	assert.equal(calls(), 2);
	poller.stop();
});

test('stop cancels the pending tick', async () => {
	const { poller, calls } = counting();
	poller.start();
	poller.stop();
	mock.timers.tick(INTERVAL * 3);
	await settle();
	assert.equal(calls(), 0);
	assert.equal(poller.scheduled, false);
});

test('hidden stops ticking, visible again ticks at once and resumes', async () => {
	const { poller, calls } = counting();
	poller.start();
	poller.setHidden(true);
	mock.timers.tick(INTERVAL * 5);
	await settle();
	assert.equal(calls(), 0);
	assert.equal(poller.scheduled, false);

	poller.setHidden(false);
	await settle();
	assert.equal(calls(), 1);
	mock.timers.tick(INTERVAL);
	await settle();
	assert.equal(calls(), 2);
	poller.stop();
});

test('showing again while a tick is in flight does not start a second chain', async () => {
	let release: () => void = () => {};
	let calls = 0;
	const poller = new Poller(() => {
		calls++;
		return new Promise<void>((resolve) => (release = resolve));
	}, INTERVAL);
	poller.start();
	mock.timers.tick(INTERVAL);
	poller.setHidden(true);
	poller.setHidden(false);
	assert.equal(calls, 1);
	release();
	await settle();
	assert.equal(poller.scheduled, true);
	mock.timers.tick(INTERVAL);
	assert.equal(calls, 2);
	poller.stop();
});

test('a poller started while hidden stays idle until shown', async () => {
	const { poller, calls } = counting();
	poller.setHidden(true);
	poller.start();
	assert.equal(poller.scheduled, false);
	poller.setHidden(false);
	await settle();
	assert.equal(calls(), 1);
	assert.equal(poller.scheduled, true);
	poller.stop();
});

test('a tick still running when restarted does not schedule a second chain', async () => {
	let release: () => void = () => {};
	let calls = 0;
	const poller = new Poller(() => {
		calls++;
		return new Promise<void>((resolve) => (release = resolve));
	}, INTERVAL);
	poller.start();
	mock.timers.tick(INTERVAL);
	const releaseOld = release;
	poller.start();
	releaseOld();
	await settle();
	mock.timers.tick(INTERVAL);
	assert.equal(calls, 2);
	release();
	await settle();
	mock.timers.tick(INTERVAL);
	assert.equal(calls, 3);
	poller.stop();
});
