import { describe, expect, it, vi } from 'vitest';

import type { ShimStatus } from '../api/index.js';
import { deferred, recordingNotifier, testBackend, testRunner } from '../testing/harness.js';
import { DiagnosticsStore } from './diagnostics.svelte.js';

const status = (installed: boolean, onPath: boolean): ShimStatus => ({ shimDir: 'C:\\Lampo\\bin', installed, onPath, shadowed: [] });

describe('DiagnosticsStore shims', () => {
	it('uninstalling shows the new status and is busy meanwhile', async () => {
		const pending = deferred<ShimStatus>();
		const { backend } = testBackend({ shimStatus: async () => status(true, true), uninstallShims: () => pending.promise });
		const store = new DiagnosticsStore(backend, testRunner());
		await store.loadShims();

		const action = store.uninstallShims();
		expect(store.shimsBusy).toBe('uninstall');
		pending.resolve(status(false, false));
		const result = await action;

		expect(result.ok).toBe(true);
		expect(store.shimsBusy).toBeNull();
		expect(store.shims.data).toEqual(status(false, false));
	});

	it('re-runs a doctor report already shown, whose PATH findings changed', async () => {
		const runDoctor = vi.fn(testBackend().backend.commands.runDoctor);
		const { backend } = testBackend({ runDoctor, uninstallShims: async () => status(false, false) });
		const store = new DiagnosticsStore(backend, testRunner());
		await store.runDoctor();

		await store.uninstallShims();

		expect(runDoctor).toHaveBeenCalledTimes(2);
	});

	it('does not run the doctor when no report was shown', async () => {
		const runDoctor = vi.fn(testBackend().backend.commands.runDoctor);
		const { backend } = testBackend({ runDoctor, installShims: async () => status(true, true) });
		const store = new DiagnosticsStore(backend, testRunner());

		await store.installShims();

		expect(store.shims.data).toEqual(status(true, true));
		expect(runDoctor).not.toHaveBeenCalled();
	});

	it('a failed uninstall keeps the previous status and reports the error', async () => {
		const notifier = recordingNotifier();
		const { backend } = testBackend({
			shimStatus: async () => status(true, true),
			uninstallShims: async () => {
				throw { code: 'io', message: 'deleting php.exe: access denied' };
			}
		});
		const store = new DiagnosticsStore(backend, testRunner(notifier));
		await store.loadShims();

		const result = await store.uninstallShims();

		expect(result.ok).toBe(false);
		expect(store.shimsBusy).toBeNull();
		expect(store.shims.data).toEqual(status(true, true));
		expect(notifier.errors).toHaveLength(1);
	});
});
