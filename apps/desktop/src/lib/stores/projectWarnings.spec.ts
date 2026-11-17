import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ProjectWarning } from '../api/index.js';
import { seedProjects } from '../backend/mock/seed.js';
import { describeProjectWarning } from '../projects/warnings.js';
import type { Notifier } from '../services/notifier.js';
import { testBackend, testRunner } from '../testing/harness.js';
import { ProjectDraft } from './projectDraft.svelte.js';
import { ProjectsStore } from './projects.svelte.js';

/** Remembers the successes and the warnings it is told. */
function notices(): Notifier & { successes: string[]; warnings: string[] } {
	const successes: string[] = [];
	const warnings: string[] = [];
	return { successes, warnings, success: (m) => void successes.push(m), info: () => {}, warning: (m) => void warnings.push(m), error: () => {} };
}

function shop() {
	return seedProjects().find((p) => p.id === 'shop')!;
}

describe('the warnings of a saved project', () => {
	beforeEach(() => vi.useFakeTimers());
	afterEach(() => vi.useRealTimers());

	it('a PHP 7.2 switch on MySQL 8.4 warns in Italian, with the 5.7 alternative', async () => {
		const notifier = notices();
		const store = new ProjectsStore(testBackend().backend, testRunner(notifier));

		const saving = store.setPhp('shop', '7.2');
		await vi.advanceTimersByTimeAsync(1000);
		const result = await saving;

		expect(result.ok && result.value.warnings.map((w) => w.code)).toEqual(['legacy-php-mysql-auth']);
		expect(notifier.warnings).toHaveLength(1);
		expect(notifier.warnings[0]).toMatch(/^PHP 7\.2 non supporta l’autenticazione predefinita di MySQL 8\.4/);
		expect(notifier.warnings[0]).toContain('MySQL 5.7');
	});

	it('a save that needs nothing is a plain confirmation', async () => {
		const notifier = notices();
		const store = new ProjectsStore(testBackend().backend, testRunner(notifier));
		const draft = new ProjectDraft(shop());
		draft.name = 'Shop renamed';

		const saving = store.update(draft.toRequest());
		await vi.advanceTimersByTimeAsync(1000);
		await saving;

		expect(notifier.warnings).toEqual([]);
		expect(notifier.successes).toEqual(['Progetto salvato.']);
	});

	it('a code this GUI does not know shows the backend’s text', () => {
		const unknown = { code: 'from-a-newer-app', message: 'English text' } as unknown as ProjectWarning;
		expect(describeProjectWarning(shop(), unknown)).toBe('English text');
	});
});
