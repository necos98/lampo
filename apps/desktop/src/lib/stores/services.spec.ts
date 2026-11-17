import { describe, expect, it } from 'vitest';

import type { AppError, SyncReport } from '../api/index.js';
import { messages as m } from '../messages/index.js';
import { deferred, recordingNotifier, testBackend, testRunner } from '../testing/harness.js';
import { ServicesStore } from './services.svelte.js';

const MYSQL = 'mysql-8.4';
const STOPPED: SyncReport = { started: [], stopped: [MYSQL], restarted: [], hostsFileUpdated: false, warnings: [] };
/** What the app answers for a start that a stop gave up. */
const GIVEN_UP: AppError = { code: 'cancelled', message: 'cancelled' };

/** A store whose start of MySQL stays pending until the test settles it; `stop` answers the stop. */
function storeWithPendingStart(stop: () => Promise<SyncReport> = async () => STOPPED) {
	const start = deferred<SyncReport>();
	const notifier = recordingNotifier();
	const env = testBackend({ startServices: () => start.promise, stopServices: stop });
	return { store: new ServicesStore(env.backend, testRunner(notifier)), start, notifier };
}

describe('ServicesStore actions in flight', () => {
	it('lets a stop overtake a start, and only a stop', () => {
		const { store } = storeWithPendingStart();
		void store.act('start', [MYSQL]);

		expect(store.busy[MYSQL]).toBe('start');
		expect(store.canAct('stop', [MYSQL])).toBe(true);
		expect(store.canAct('restart', [MYSQL])).toBe(false);
		expect(store.canAct('start', [MYSQL])).toBe(false);
	});

	it('takes a start given up by the stop for no failure', async () => {
		const { store, start, notifier } = storeWithPendingStart();
		const starting = store.act('start', [MYSQL]);

		await store.act('stop', [MYSQL]);
		start.reject(GIVEN_UP);
		await starting;

		expect(store.busy).toEqual({});
		expect(store.failures[MYSQL]).toBeUndefined();
		expect(notifier.errors).toEqual([]);
	});

	it('leaves the flag to the stop while it runs', async () => {
		const stop = deferred<SyncReport>();
		const { store, start } = storeWithPendingStart(() => stop.promise);
		const starting = store.act('start', [MYSQL]);
		const stopping = store.act('stop', [MYSQL]);

		start.reject(GIVEN_UP);
		await starting;
		expect(store.busy[MYSQL]).toBe('stop');

		stop.resolve(STOPPED);
		await stopping;
		expect(store.busy).toEqual({});
	});
});

describe('ServicesStore actions on every service', () => {
	it('lets «Avvia/Ferma tutti» run only when nothing is in flight, as the page buttons', () => {
		const { store } = storeWithPendingStart();
		expect(store.canAct('start', [])).toBe(true);

		void store.act('start', [MYSQL]);

		expect(store.anyBusy).toBe(true);
		expect(store.canAct('start', [])).toBe(false);
		expect(store.canAct('stop', [])).toBe(false);
	});

	it('offers a «Riprova» that does nothing while the services are busy again', async () => {
		const calls: string[][] = [];
		const pending = deferred<SyncReport>();
		const failure: AppError = { code: 'process-failed', message: 'apache failed' };
		const notifier = recordingNotifier();
		const env = testBackend({
			startServices: async (request) => {
				calls.push(request?.services ?? []);
				if (calls.length === 1) throw failure;
				return pending.promise;
			}
		});
		const store = new ServicesStore(env.backend, testRunner(notifier));

		await store.act('start', [MYSQL]);
		const retry = notifier.errorNotices[0]?.action;
		expect(retry?.label).toBe(m.common.errorActions.retry);

		retry?.run();
		retry?.run();
		expect(calls).toEqual([[MYSQL], [MYSQL]]);

		pending.resolve({ ...STOPPED, stopped: [], started: [MYSQL] });
	});
});
