import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { AppError, TaskFinishedPayload } from '../../api/index.js';
import { localizeError } from '../../services/localizeError.js';
import { it as m } from '../../messages/it.js';
import { createMockBackend } from './index.js';

/** Longer than any mock delay or task. */
const SETTLE_MS = 5_000;

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

/** The rejection of `call`, once the mock's delays have run. */
async function rejection(call: Promise<unknown>): Promise<AppError> {
	const caught = call.then(
		() => expect.unreachable('the call should fail'),
		(e: AppError) => e
	);
	await vi.advanceTimersByTimeAsync(SETTLE_MS);
	return caught;
}

describe('mock backend errors carry the details of the real backend', () => {
	it('not-found names the entity and the id', async () => {
		const backend = createMockBackend({ fresh: false });

		const error = await rejection(backend.commands.getProject('nope'));

		expect(error).toEqual({ code: 'not-found', message: 'project `nope` not found', details: { entity: 'project', id: 'nope' } });
		expect(localizeError(error, m)).toBe('Il progetto «nope» non esiste.');
	});

	it('a removed worktree environment that is gone is named too', async () => {
		const backend = createMockBackend({ fresh: false });

		const error = await rejection(backend.commands.removeWorktree({ target: 'shop@gone', dropDatabase: false }));

		expect(error.details).toEqual({ entity: 'worktree environment', id: 'shop@gone' });
	});

	it('package-missing names the package', async () => {
		const backend = createMockBackend({ fresh: false });

		const error = await rejection(backend.commands.listPhpExtensions('5.3'));

		expect(error).toMatchObject({ code: 'package-missing', details: { package: 'PHP 5.3' } });
		expect(localizeError(error, m)).toBe(m.errorDetails.packageMissing('PHP 5.3'));
	});

	it('uninstalling the last build a project needs fails its task with who uses it', async () => {
		const backend = createMockBackend({ fresh: false });
		const listing = backend.commands.listInstalledPackages();
		await vi.advanceTimersByTimeAsync(SETTLE_MS);
		const used = (await listing).find((p) => p.id.kind === 'php' && p.usedBy.length > 0);
		expect(used, 'the seed has a PHP in use').toBeDefined();
		const finished: TaskFinishedPayload[] = [];
		await backend.events.onTaskFinished((p) => finished.push(p));

		const started = backend.commands.uninstallPackage(used!.id);
		await vi.advanceTimersByTimeAsync(SETTLE_MS);
		const { taskId } = await started;

		const outcome = finished.find((f) => f.taskId === taskId)?.outcome;
		expect(outcome?.status).toBe('failed');
		const error = outcome?.status === 'failed' ? outcome.error : null;
		expect(error?.code).toBe('conflict');
		expect(error?.details).toMatchObject({ usedBy: used!.usedBy });
		expect(localizeError(error!, m)).toMatch(/è in uso da/);
	});
});
