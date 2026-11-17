import { describe, expect, it, vi } from 'vitest';

import type { DefenderExclusionSuggestion, TaskOutcome } from '../api/index.js';
import { installTaskKey, uninstallTaskKey } from '../packages/grouping.js';
import type { Notifier, NotifierAction } from '../services/notifier.js';
import { changedPhp, DefenderExclusionNotices, type SuggestionTexts } from './defenderExclusionNotices.js';
import type { TaskMeta, TaskState } from './tasks.svelte.js';

const TEXTS: SuggestionTexts = { add: 'new PHP not covered', remove: 'uninstalled PHP still excluded', action: { add: 'Update', remove: 'Remove' } };

const ADD: DefenderExclusionSuggestion = {
	change: 'add',
	processes: ['C:\\Users\\dev\\AppData\\Local\\Lampo\\packages\\php-8.5\\php-cgi.exe'],
	request: { change: 'add', php: true, mysql: false, projects: false, projectsFolder: null, uninstalled: false },
	command: 'lampo defender exclusion add --php --allow-elevation'
};
const REMOVE: DefenderExclusionSuggestion = {
	change: 'remove',
	processes: ['C:\\Users\\dev\\AppData\\Local\\Lampo\\packages\\php-8.2\\php-cgi.exe'],
	request: { change: 'remove', php: false, mysql: false, projects: false, projectsFolder: null, uninstalled: true },
	command: 'lampo defender exclusion remove --uninstalled --allow-elevation'
};

const SUCCEEDED: TaskOutcome = { status: 'succeeded' };
const FAILED: TaskOutcome = { status: 'failed', error: { code: 'network', message: 'offline' } };

const task = (meta: TaskMeta | null, outcome: TaskOutcome | null = SUCCEEDED): TaskState => ({ taskId: 't1', meta, progress: null, subjects: {}, outcome, lines: [] });
const installed = (kind: string, outcome: TaskOutcome = SUCCEEDED) => task({ kind: 'install', key: installTaskKey(kind, '8.5', null) }, outcome);
const uninstalled = (kind: string, phpVersion: string | null = null) => task({ kind: 'uninstall', key: uninstallTaskKey({ kind, version: '8.2.27', phpVersion }) });

/** The notices started on a hand-fired task bus, recording the notices shown and the requests made. */
async function started(suggestions: () => Promise<DefenderExclusionSuggestion[]>) {
	const shown: { message: string; action?: NotifierAction }[] = [];
	const notifier: Notifier = { success: vi.fn(), info: (message, action) => void shown.push({ message, action }), warning: vi.fn(), error: vi.fn() };
	let listener: ((task: TaskState) => void) | null = null;
	const apply = vi.fn();
	const asked = vi.fn(suggestions);
	const sources = {
		onTaskFinished: (l: (task: TaskState) => void) => {
			listener = l;
			return () => void (listener = null);
		},
		suggestions: asked,
		apply
	};
	const stop = await new DefenderExclusionNotices(sources, notifier, TEXTS).start();
	/** Fires `t`, then lets any proposal it asked for settle. */
	const finish = async (t: TaskState) => {
		listener?.(t);
		await Promise.allSettled(asked.mock.results.map((r) => r.value));
		await Promise.resolve();
	};
	return { shown, apply, asked, finish, stop };
}

describe('changedPhp', () => {
	it('is a finished install or uninstall of a PHP', () => {
		expect(changedPhp(installed('php'))).toBe(true);
		expect(changedPhp(uninstalled('php'))).toBe(true);
	});

	it('is not another package, an add-on built for a PHP, an unfinished or failed task, or another task', () => {
		expect(changedPhp(installed('mysql'))).toBe(false);
		expect(changedPhp(uninstalled('php-redis', '8.3'))).toBe(false);
		expect(changedPhp(uninstalled('xdebug', '8.3'))).toBe(false);
		expect(changedPhp(installed('php', { status: 'cancelled' }))).toBe(false);
		expect(changedPhp(installed('php', FAILED))).toBe(false);
		expect(changedPhp(task({ kind: 'install', key: installTaskKey('php', '8.5', null) }, null))).toBe(false);
		expect(changedPhp(task({ kind: 'first-run', key: 'first-run' }))).toBe(false);
		expect(changedPhp(task(null))).toBe(false);
	});
});

describe('DefenderExclusionNotices', () => {
	it('after a PHP install proposes each change with a button that makes it', async () => {
		const { shown, apply, finish } = await started(async () => [ADD, REMOVE]);

		await finish(installed('php'));

		expect(shown.map((s) => [s.message, s.action?.label])).toEqual([
			[TEXTS.add, TEXTS.action.add],
			[TEXTS.remove, TEXTS.action.remove]
		]);
		expect(apply).not.toHaveBeenCalled();
		shown[1].action?.run();
		expect(apply).toHaveBeenCalledExactlyOnceWith(REMOVE);
	});

	it('shows nothing when the backend proposes nothing', async () => {
		const { shown, asked, finish } = await started(async () => []);
		await finish(uninstalled('php'));
		expect(asked).toHaveBeenCalledTimes(1);
		expect(shown).toEqual([]);
	});

	it('never asks after another package or a failed install', async () => {
		const { asked, finish } = await started(async () => [ADD]);
		await finish(installed('mysql'));
		await finish(installed('php', FAILED));
		expect(asked).not.toHaveBeenCalled();
	});

	it('a proposal that cannot be read shows nothing', async () => {
		const { shown, finish } = await started(async () => {
			throw { code: 'internal', message: 'ipc closed' };
		});
		await finish(installed('php'));
		expect(shown).toEqual([]);
	});

	it('stops listening when disposed', async () => {
		const { asked, finish, stop } = await started(async () => [ADD]);
		stop();
		await finish(installed('php'));
		expect(asked).not.toHaveBeenCalled();
	});
});
