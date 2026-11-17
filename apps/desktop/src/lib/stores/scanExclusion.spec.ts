import { describe, expect, it, vi } from 'vitest';

import type { ScanExclusionOutcome, ScanExclusionResult } from '../api/index.js';
import { messages as m } from '../messages/index.js';
import { deferred, recordingNotifier, testBackend, testRunner } from '../testing/harness.js';
import { ScanExclusionStore } from './scanExclusion.svelte.js';

const FOLDER = 'C:\\Users\\dev\\AppData\\Local\\Lampo\\mysql';
const result = (outcome: ScanExclusionOutcome, change: 'add' | 'remove' = 'add'): ScanExclusionResult => ({ change, outcome, folder: FOLDER });

describe('ScanExclusionStore', () => {
	it('loads the folder without changing anything', async () => {
		const change = vi.fn();
		const { backend } = testBackend({ changeMysqlScanExclusion: change });
		const store = new ScanExclusionStore(backend, testRunner());

		await store.load();

		expect(store.info.data).toEqual({ folder: FOLDER });
		expect(store.last).toBeNull();
		expect(change).not.toHaveBeenCalled();
	});

	it('adding is busy meanwhile, then remembers the request and says it is done', async () => {
		const pending = deferred<ScanExclusionResult>();
		const change = vi.fn(() => pending.promise);
		const notifier = recordingNotifier();
		const { backend } = testBackend({ changeMysqlScanExclusion: change });
		const store = new ScanExclusionStore(backend, testRunner(notifier));

		const action = store.add();
		expect(store.busy).toBe('add');
		pending.resolve(result('applied'));
		await action;

		expect(change).toHaveBeenCalledWith('add');
		expect(store.busy).toBeNull();
		expect(store.last).toEqual(result('applied'));
		expect(notifier.successes).toEqual([m.settings.defender.done.add]);
	});

	it('removing asks for the removal', async () => {
		const change = vi.fn(async () => result('applied', 'remove'));
		const notifier = recordingNotifier();
		const { backend } = testBackend({ changeMysqlScanExclusion: change });

		await new ScanExclusionStore(backend, testRunner(notifier)).remove();

		expect(change).toHaveBeenCalledWith('remove');
		expect(notifier.successes).toEqual([m.settings.defender.done.remove]);
	});

	it.each(['cancelled', 'defender-unavailable', 'refused', 'not-confirmed'] as const)('%s is a warning that says why nothing changed', async (outcome) => {
		const notifier = recordingNotifier();
		const { backend } = testBackend({ changeMysqlScanExclusion: async () => result(outcome) });
		const store = new ScanExclusionStore(backend, testRunner(notifier));

		const action = await store.add();

		expect(action.ok).toBe(true);
		expect(notifier.warnings).toEqual([m.settings.defender.outcome[outcome]]);
		expect(notifier.successes).toEqual([]);
		expect(notifier.errors).toEqual([]);
		expect(store.last?.outcome).toBe(outcome);
	});

	it('a failure is an error and keeps the previous request', async () => {
		const notifier = recordingNotifier();
		const { backend } = testBackend({
			changeMysqlScanExclusion: async () => {
				throw { code: 'validation', message: 'only the MySQL data folder of a Windows user profile can be excluded' };
			}
		});
		const store = new ScanExclusionStore(backend, testRunner(notifier));

		const action = await store.add();

		expect(action.ok).toBe(false);
		expect(store.busy).toBeNull();
		expect(store.last).toBeNull();
		expect(notifier.errors).toHaveLength(1);
	});
});
