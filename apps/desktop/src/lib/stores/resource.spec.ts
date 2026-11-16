import { describe, expect, it } from 'vitest';

import { deferred } from '../testing/harness.js';
import { Resource } from './resource.svelte.js';

describe('Resource', () => {
	it('starts idle and loading, then shows the loaded value', async () => {
		const resource = new Resource(async () => 'a');
		expect(resource.status).toBe('idle');
		expect(resource.loading).toBe(true);

		await resource.load();

		expect(resource.data).toBe('a');
		expect(resource.status).toBe('ready');
		expect(resource.loading).toBe(false);
	});

	it('ignores a slow response that was overtaken by a newer load', async () => {
		const slow = deferred<string>();
		const fast = deferred<string>();
		const responses = [slow, fast];
		const resource = new Resource(() => responses.shift()!.promise);

		const first = resource.load();
		const second = resource.load();
		fast.resolve('new');
		await second;
		slow.resolve('old');
		await first;

		expect(resource.data).toBe('new');
		expect(resource.refreshing).toBe(false);
	});

	it('ignores a failure of a load that was overtaken', async () => {
		const slow = deferred<string>();
		const fast = deferred<string>();
		const responses = [slow, fast];
		const resource = new Resource(() => responses.shift()!.promise);

		const first = resource.load();
		const second = resource.load();
		fast.resolve('new');
		await second;
		slow.reject({ code: 'internal', message: 'late' });
		await first;

		expect(resource.error).toBeNull();
		expect(resource.status).toBe('ready');
	});

	it('does not let a response that started before a set() overwrite it', async () => {
		const pending = deferred<string>();
		const resource = new Resource(() => pending.promise);

		const load = resource.load();
		resource.set('authoritative');
		pending.resolve('stale');
		await load;

		expect(resource.data).toBe('authoritative');
		expect(resource.status).toBe('ready');
	});

	it('keeps the data and reports the error when a refresh fails', async () => {
		let fail = false;
		const resource = new Resource(async () => {
			if (fail) throw { code: 'internal', message: 'boom' };
			return 'kept';
		});
		await resource.load();

		fail = true;
		await resource.load();

		expect(resource.data).toBe('kept');
		expect(resource.status).toBe('ready');
		expect(resource.error?.message).toBe('boom');
		expect(resource.refreshing).toBe(false);
	});

	it('goes to the error state when the first load fails', async () => {
		const resource = new Resource<string>(async () => {
			throw { code: 'internal', message: 'boom' };
		});

		await resource.load();

		expect(resource.status).toBe('error');
		expect(resource.data).toBeNull();
	});

	it('flags refreshing, not loading, while reloading shown data', async () => {
		const pending = deferred<string>();
		let calls = 0;
		const resource = new Resource(() => (calls++ === 0 ? Promise.resolve('first') : pending.promise));
		await resource.load();

		const reload = resource.load();
		expect(resource.refreshing).toBe(true);
		expect(resource.status).toBe('ready');
		pending.resolve('second');
		await reload;

		expect(resource.data).toBe('second');
		expect(resource.refreshing).toBe(false);
	});

	it('ensure() loads once and reuses the result', async () => {
		let calls = 0;
		const resource = new Resource(async () => ++calls);

		await Promise.all([resource.ensure(), resource.ensure()]);
		await resource.ensure();

		expect(calls).toBe(1);
	});

	it('patch() applies on the data and drops a response already in flight, then reloads', async () => {
		const stale = deferred<string[]>();
		const fresh = deferred<string[]>();
		const responses = [Promise.resolve(['a']), stale.promise, fresh.promise];
		const resource = new Resource(() => responses.shift()!);
		await resource.load();

		const reload = resource.load();
		resource.patch((items) => [...items, 'event']);
		expect(resource.data).toEqual(['a', 'event']);
		stale.resolve(['a']);
		await reload;
		expect(resource.data).toEqual(['a', 'event']);

		fresh.resolve(['a', 'event', 'other']);
		await Promise.resolve();
		await Promise.resolve();
		expect(resource.data).toEqual(['a', 'event', 'other']);
	});

	it('patch() before any data triggers a load instead', async () => {
		const resource = new Resource(async () => ['x']);

		resource.patch((items) => [...items, 'y']);
		await Promise.resolve();
		await Promise.resolve();

		expect(resource.data).toEqual(['x']);
	});
});
