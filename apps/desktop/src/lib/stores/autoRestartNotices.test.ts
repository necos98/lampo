import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { ServiceAutoRestartPayload } from '../api/index.js';
import type { Notifier } from '../services/notifier.js';
import { AutoRestartNotices, type AutoRestartTexts } from './autoRestartNotices.ts';

const TEXTS: AutoRestartTexts = {
	scheduled: (label, attempt, max, seconds) => `${label} ${attempt}/${max} in ${seconds}s`,
	restarted: (label) => `${label} up`,
	gaveUp: (label, attempts) => `${label} gave up after ${attempts}`
};

/** A notifier recording `level: message (description)`, and an event bus fired by hand. */
async function started() {
	const shown: string[] = [];
	const notifier: Notifier = {
		success: (message) => void shown.push(`success: ${message}`),
		info: (message) => void shown.push(`info: ${message}`),
		warning: (message) => void shown.push(`warning: ${message}`),
		error: (message, notice) => void shown.push(`error: ${message} (${notice?.description})`)
	};
	let handler: ((p: ServiceAutoRestartPayload) => void) | null = null;
	const events = {
		onServiceAutoRestart(h: (p: ServiceAutoRestartPayload) => void) {
			handler = h;
			return Promise.resolve(() => void (handler = null));
		}
	};
	const labels: Record<string, string> = { 'mysql-8.4': 'MySQL 8.4' };
	const stop = await new AutoRestartNotices(events, notifier, TEXTS, (id) => labels[id] ?? id).start();
	const fire = (p: ServiceAutoRestartPayload) => handler?.(p);
	return { shown, fire, stop };
}

test('every step of a restart is a notice naming the service', async () => {
	const { shown, fire } = await started();
	fire({ service: 'mysql-8.4', step: { type: 'scheduled', attempt: 1, maxAttempts: 3, delayMs: 2000 } });
	fire({ service: 'mysql-8.4', step: { type: 'restarted', attempt: 1 } });
	fire({ service: 'redis', step: { type: 'gave-up', attempts: 3, message: 'port 6379 is already in use' } });

	assert.deepEqual(shown, ['warning: MySQL 8.4 1/3 in 2s', 'success: MySQL 8.4 up', 'error: redis gave up after 3 (port 6379 is already in use)']);
});

test('no notices once stopped', async () => {
	const { shown, fire, stop } = await started();
	stop();
	fire({ service: 'redis', step: { type: 'restarted', attempt: 2 } });

	assert.deepEqual(shown, []);
});
