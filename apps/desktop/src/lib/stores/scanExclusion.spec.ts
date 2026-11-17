import { describe, expect, it, vi } from 'vitest';

import type { DefenderExclusionCommand, DefenderExclusionRequest, DefenderExclusionResult, ScanExclusionChange, ScanExclusionOutcome } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import type { Notifier } from '../services/notifier.js';
import { deferred, recordingNotifier, testBackend, testRunner } from '../testing/harness.js';
import type { CopyText } from './logFiles.js';
import { exclusionRequest, itCommandsText, needsItCommands, ScanExclusionStore, type DefenderTarget } from './scanExclusion.svelte.js';

const FOLDER = 'C:\\Users\\dev\\AppData\\Local\\Lampo\\mysql';
const PROCESSES = ['C:\\Users\\dev\\AppData\\Local\\Lampo\\packages\\apache-2.4\\bin\\httpd.exe', 'C:\\Users\\dev\\AppData\\Local\\Lampo\\packages\\php-8.3\\php-cgi.exe'];
const PROJECTS = 'C:\\Users\\dev\\Desktop\\Progetti';

const result = (target: DefenderTarget, outcome: ScanExclusionOutcome, change: ScanExclusionChange = 'add'): DefenderExclusionResult => ({
	change,
	outcome,
	folder: target === 'mysql' ? FOLDER : null,
	processes: target === 'php' ? PROCESSES : [],
	projects: target === 'projects' ? PROJECTS : null
});

/** The store with `notifier`, copying through `copyText`. */
const storeFor = (backend: Backend, notifier: Notifier = recordingNotifier(), copyText: CopyText = async () => true) =>
	new ScanExclusionStore(backend, testRunner(notifier), notifier, copyText);

/** A backend that answers every request with `outcome` for what it names. */
const answering =
	(outcome: ScanExclusionOutcome) =>
	async ({ change, php, projects }: DefenderExclusionRequest) =>
		result(php ? 'php' : projects ? 'projects' : 'mysql', outcome, change);

describe('ScanExclusionStore', () => {
	it('loads what each option covers without changing anything', async () => {
		const change = vi.fn();
		const { backend } = testBackend({ changeDefenderExclusion: change });
		const store = storeFor(backend);

		await store.load();

		expect(store.info.data).toEqual({ folder: FOLDER, processes: PROCESSES, projectsFolder: PROJECTS });
		expect(store.last).toEqual({ php: null, mysql: null, projects: null });
		expect(change).not.toHaveBeenCalled();
	});

	it.each([
		['php', { change: 'add', php: true, mysql: false, projects: false, projectsFolder: null }],
		['mysql', { change: 'add', php: false, mysql: true, projects: false, projectsFolder: null }]
	] as const)('adding %s asks for that exclusion only, is busy meanwhile, then remembers it and says it is done', async (target, request) => {
		const pending = deferred<DefenderExclusionResult>();
		const change = vi.fn(() => pending.promise);
		const notifier = recordingNotifier();
		const { backend } = testBackend({ changeDefenderExclusion: change });
		const store = storeFor(backend, notifier);

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

		await storeFor(backend, notifier).remove(target);

		expect(change).toHaveBeenCalledExactlyOnceWith(exclusionRequest(target, 'remove'));
		expect(notifier.successes).toEqual([m.settings.defender[target].done.remove]);
	});

	it('the projects folder goes with a projects request only, trimmed', async () => {
		const change = vi.fn(answering('applied'));
		const notifier = recordingNotifier();
		const { backend } = testBackend({ changeDefenderExclusion: change });
		const store = storeFor(backend, notifier);

		await store.add('projects', `  ${PROJECTS} `);
		await store.remove('projects', PROJECTS);
		await store.add('php', PROJECTS);

		expect(change.mock.calls.map(([request]) => request)).toEqual([
			{ change: 'add', php: false, mysql: false, projects: true, projectsFolder: PROJECTS },
			{ change: 'remove', php: false, mysql: false, projects: true, projectsFolder: PROJECTS },
			{ change: 'add', php: true, mysql: false, projects: false, projectsFolder: null }
		]);
		expect(store.last.projects).toEqual(result('projects', 'applied', 'remove'));
		expect(notifier.successes).toEqual([m.settings.defender.projects.done.add, m.settings.defender.projects.done.remove, m.settings.defender.php.done.add]);
	});

	it('a projects request without a folder lets the backend suggest one', () => {
		expect(exclusionRequest('projects', 'add')).toEqual({ change: 'add', php: false, mysql: false, projects: true, projectsFolder: null });
	});

	it('each option keeps its own last request', async () => {
		const { backend } = testBackend({ changeDefenderExclusion: answering('applied') });
		const store = storeFor(backend);

		await store.add('php');
		await store.remove('mysql');

		expect(store.last).toEqual({ php: result('php', 'applied'), mysql: result('mysql', 'applied', 'remove'), projects: null });
	});

	it('a request while another prompt is open is ignored', async () => {
		const pending = deferred<DefenderExclusionResult>();
		const change = vi.fn(() => pending.promise);
		const { backend } = testBackend({ changeDefenderExclusion: change });
		const store = storeFor(backend);

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
		const store = storeFor(backend, notifier);

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
		const store = storeFor(backend, notifier);

		const action = await store.add('php');

		expect(action?.ok).toBe(false);
		expect(store.busy).toBeNull();
		expect(store.last).toEqual({ php: null, mysql: null, projects: null });
		expect(notifier.errors).toHaveLength(1);
	});
});

describe('commands for whoever manages the PC', () => {
	const command = (kind: DefenderExclusionCommand['kind'], path: string): DefenderExclusionCommand => ({
		kind,
		path,
		command: `Add-MpPreference ${kind === 'path' ? '-ExclusionPath' : '-ExclusionProcess'} '${path}'`
	});

	it('are offered first after a refused or unconfirmed addition only', () => {
		for (const outcome of ['refused', 'not-confirmed'] as const) {
			expect(needsItCommands(result('php', outcome))).toBe(true);
			expect(needsItCommands(result('php', outcome, 'remove'))).toBe(false);
		}
		for (const outcome of ['applied', 'cancelled', 'defender-unavailable'] as const) expect(needsItCommands(result('php', outcome))).toBe(false);
		expect(needsItCommands(null)).toBe(false);
	});

	it('are the lines the CLI prints, one per exclusion', () => {
		const commands = [command('path', FOLDER), ...PROCESSES.map((exe) => command('process', exe))];
		expect(itCommandsText(commands)).toBe(commands.map((c) => c.command).join('\n'));
		expect(itCommandsText([])).toBe('');
	});

	it.each([
		['php', undefined, { change: 'add', php: true, mysql: false, projects: false, projectsFolder: null }],
		['projects', ` ${PROJECTS} `, { change: 'add', php: false, mysql: false, projects: true, projectsFolder: PROJECTS }]
	] as const)('copying %s asks for the addition’s commands and copies their text, without any change', async (target, folder, request) => {
		const commands = [command('process', PROCESSES[0])];
		const build = vi.fn(async () => commands);
		const change = vi.fn();
		const copyText = vi.fn(async () => true);
		const notifier = recordingNotifier();
		const { backend } = testBackend({ defenderExclusionCommands: build, changeDefenderExclusion: change });

		expect(await storeFor(backend, notifier, copyText).copyItCommands(target, folder)).toBe(true);

		expect(build).toHaveBeenCalledExactlyOnceWith(request);
		expect(copyText).toHaveBeenCalledExactlyOnceWith(itCommandsText(commands));
		expect(notifier.successes).toEqual([m.settings.defender.itCommands.copied]);
		expect(change).not.toHaveBeenCalled();
	});

	it('the mock builds the same lines for the same request', async () => {
		const { backend } = testBackend();
		const commands = await backend.commands.defenderExclusionCommands(exclusionRequest('php', 'add'));
		expect(commands).toEqual(PROCESSES.map((exe) => command('process', exe)));
	});

	it('a refused clipboard is an error', async () => {
		const notifier = recordingNotifier();
		const { backend } = testBackend();

		expect(await storeFor(backend, notifier, async () => false).copyItCommands('mysql')).toBe(false);

		expect(notifier.errors).toEqual([m.common.copyFailed]);
		expect(notifier.successes).toEqual([]);
	});

	it('a backend refusal is told and nothing is copied', async () => {
		const notifier = recordingNotifier();
		const copyText = vi.fn(async () => true);
		const { backend } = testBackend({
			defenderExclusionCommands: async () => {
				throw { code: 'validation', message: 'AppData and the folders in it are never excluded' };
			}
		});

		expect(await storeFor(backend, notifier, copyText).copyItCommands('projects', 'C:\Users\dev\AppData')).toBe(false);

		expect(copyText).not.toHaveBeenCalled();
		expect(notifier.errors).toHaveLength(1);
	});
});
