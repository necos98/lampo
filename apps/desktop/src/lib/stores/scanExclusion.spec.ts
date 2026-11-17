import { describe, expect, it, vi } from 'vitest';

import type { DefenderExclusionRequest, DefenderExclusionResult, ScanExclusionChange, ScanExclusionOutcome } from '../api/index.js';
import { messages as m } from '../messages/index.js';
import { deferred, recordingNotifier, testBackend, testRunner } from '../testing/harness.js';
import { ScanExclusionStore, type DefenderTarget } from './scanExclusion.svelte.js';

const FOLDER = 'C:\\Users\\dev\\AppData\\Local\\Lampo\\mysql';
const PROCESSES = ['C:\\Users\\dev\\AppData\\Local\\Lampo\\packages\\apache-2.4\\bin\\httpd.exe', 'C:\\Users\\dev\\AppData\\Local\\Lampo\\packages\\php-8.3\\php-cgi.exe'];

const result = (target: DefenderTarget, outcome: ScanExclusionOutcome, change: ScanExclusionChange = 'add'): DefenderExclusionResult => ({
	change,
	outcome,
	folder: target === 'mysql' ? FOLDER : null,
	processes: target === 'php' ? PROCESSES : []
});

/** A backend that answers every request with `outcome` for what it names. */
const answering =
	(outcome: ScanExclusionOutcome) =>
	async ({ change, php }: DefenderExclusionRequest) =>
		result(php ? 'php' : 'mysql', outcome, change);

describe('ScanExclusionStore', () => {
	it('loads what each option covers without changing anything', async () => {
		const change = vi.fn();
		const { backend } = testBackend({ changeDefenderExclusion: change });
		const store = new ScanExclusionStore(backend, testRunner());

		await store.load();

		expect(store.info.data).toEqual({ folder: FOLDER, processes: PROCESSES });
		expect(store.last).toEqual({ php: null, mysql: null });
		expect(change).not.toHaveBeenCalled();
	});

	it.each([
		['php', { change: 'add', php: true, mysql: false }],
		['mysql', { change: 'add', php: false, mysql: true }]
	] as const)('adding %s asks for that exclusion only, is busy meanwhile, then remembers it and says it is done', async (target, request) => {
		const pending = deferred<DefenderExclusionResult>();
		const change = vi.fn(() => pending.promise);
		const notifier = recordingNotifier();
		const { backend } = testBackend({ changeDefenderExclusion: change });
		const store = new ScanExclusionStore(backend, testRunner(notifier));

		const action = store.add(target);
		expect(store.busy).toEqual({ target, change: 'add' });
		pending.resolve(result(target, 'applied'));
		await action;

		expect(change).toHaveBeenCalledExactlyOnceWith(request);
		expect(store.busy).toBeNull();
		expect(store.last[target]).toEqual(result(target, 'applied'));
		expect(notifier.successes).toEqual([m.settings.defender[target].done.add]);
	});

	it.each(['php', 'mysql'] as const)('removing %s asks for the removal', async (target) => {
		const change = vi.fn(answering('applied'));
		const notifier = recordingNotifier();
		const { backend } = testBackend({ changeDefenderExclusion: change });

		await new ScanExclusionStore(backend, testRunner(notifier)).remove(target);

		expect(change).toHaveBeenCalledExactlyOnceWith({ change: 'remove', php: target === 'php', mysql: target === 'mysql' });
		expect(notifier.successes).toEqual([m.settings.defender[target].done.remove]);
	});

	it('each option keeps its own last request', async () => {
		const { backend } = testBackend({ changeDefenderExclusion: answering('applied') });
		const store = new ScanExclusionStore(backend, testRunner());

		await store.add('php');
		await store.remove('mysql');

		expect(store.last).toEqual({ php: result('php', 'applied'), mysql: result('mysql', 'applied', 'remove') });
	});

	it('a request while another prompt is open is ignored', async () => {
		const pending = deferred<DefenderExclusionResult>();
		const change = vi.fn(() => pending.promise);
		const { backend } = testBackend({ changeDefenderExclusion: change });
		const store = new ScanExclusionStore(backend, testRunner());

		const first = store.add('php');
		expect(await store.remove('mysql')).toBeNull();
		expect(store.busy).toEqual({ target: 'php', change: 'add' });
		pending.resolve(result('php', 'applied'));
		await first;

		expect(change).toHaveBeenCalledTimes(1);
		expect(store.last.mysql).toBeNull();
	});

	it.each(['cancelled', 'defender-unavailable', 'refused', 'not-confirmed'] as const)('%s is a warning that says why nothing changed', async (outcome) => {
		const notifier = recordingNotifier();
		const { backend } = testBackend({ changeDefenderExclusion: answering(outcome) });
		const store = new ScanExclusionStore(backend, testRunner(notifier));

		const action = await store.add('php');

		expect(action?.ok).toBe(true);
		expect(notifier.warnings).toEqual([m.settings.defender.outcome[outcome]]);
		expect(notifier.successes).toEqual([]);
		expect(notifier.errors).toEqual([]);
		expect(store.last.php?.outcome).toBe(outcome);
	});

	it('a failure is an error and keeps the previous request', async () => {
		const notifier = recordingNotifier();
		const { backend } = testBackend({
			changeDefenderExclusion: async () => {
				throw { code: 'package-missing', message: 'no PHP or Apache package is installed: there is no php-cgi.exe or httpd.exe to exclude' };
			}
		});
		const store = new ScanExclusionStore(backend, testRunner(notifier));

		const action = await store.add('php');

		expect(action?.ok).toBe(false);
		expect(store.busy).toBeNull();
		expect(store.last).toEqual({ php: null, mysql: null });
		expect(notifier.errors).toHaveLength(1);
	});
});
