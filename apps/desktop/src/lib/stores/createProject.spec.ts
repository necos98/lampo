import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Addressing } from '../api/index.js';
import { SCAFFOLD_CHOICES_KEY } from '../projects/scaffoldChoices.js';
import { recordingNotifier, testBackend, testRunner, type RecordingNotifier, type TestBackend } from '../testing/harness.js';
import { CreateProjectFlow, compareMinor, folderNameFromRepoUrl, folderNameProblem, laravelNameIsValid } from './createProject.svelte.js';
import { ProjectsStore } from './projects.svelte.js';
import { ScaffoldSession } from './scaffoldSession.js';
import { TasksStore } from './tasks.svelte.js';

const TASK_ID = 'scaffold-1';
const PARENT = 'C:\\Dev';
const DETECT_MS = 500;

/** Everything the screen is wired to, with the scaffold task driven by hand. */
interface Screen {
	env: TestBackend;
	tasks: TasksStore;
	session: ScaffoldSession;
	projects: ProjectsStore;
	readyElsewhere: ReturnType<typeof vi.fn>;
	/** What the flows told the user (their error toasts, with «Riprova»). */
	notifier: RecordingNotifier;
	flow(): CreateProjectFlow;
}

let stops: Array<() => void> = [];
let stored: Record<string, string> = {};

async function screen(commands: Parameters<typeof testBackend>[0] = {}): Promise<Screen> {
	const env = testBackend({ listRunningTasks: async () => [], scaffoldProject: async () => ({ taskId: TASK_ID }), ...commands });
	const tasks = new TasksStore(env.backend);
	stops.push(await tasks.start());
	const readyElsewhere = vi.fn();
	const session = new ScaffoldSession(tasks, readyElsewhere);
	stops.push(await session.start());
	const projects = new ProjectsStore(env.backend, testRunner());
	const notifier = recordingNotifier();
	const flow = () => {
		const created = new CreateProjectFlow(env.backend, projects, testRunner(notifier), tasks, session, () => undefined as Addressing | undefined);
		stops.push(() => created.dispose());
		return created;
	};
	return { env, tasks, session, projects, readyElsewhere, notifier, flow };
}

/** Lets the pending detection (mock backend sleeps) and the promise chains settle. */
const settle = () => vi.advanceTimersByTimeAsync(DETECT_MS * 2);

function fillLaravel(flow: CreateProjectFlow): void {
	flow.setSource('laravel');
	flow.parentDir = PARENT;
	flow.folderName = 'shop2';
	flow.folderNameEdited();
}

beforeEach(() => {
	vi.useFakeTimers();
	stored = {};
	vi.stubGlobal('localStorage', {
		getItem: (key: string) => stored[key] ?? null,
		setItem: (key: string, value: string) => void (stored[key] = value)
	});
});

afterEach(() => {
	stops.forEach((stop) => stop());
	stops = [];
	vi.useRealTimers();
	vi.unstubAllGlobals();
});

describe('helpers', () => {
	it.each([
		['https://github.com/necos98/lampo.git', 'lampo'],
		['git@github.com:necos98/lampo.git/', 'lampo'],
		['  C:\\repos\\site  ', 'site'],
		['', '']
	])('folderNameFromRepoUrl(%j) is %j', (url, name) => expect(folderNameFromRepoUrl(url)).toBe(name));

	it.each([
		['', 'empty'],
		['   ', 'empty'],
		['..', 'invalid'],
		['a:b', 'invalid'],
		['name.', 'invalid'],
		['CON', 'invalid'],
		['nul.txt', 'invalid'],
		['shop', null],
		['negozio è ok', null]
	])('folderNameProblem(%j) is %j', (name, problem) => expect(folderNameProblem(name)).toBe(problem));

	it.each([
		['shop', true],
		['shop-2.x_y', true],
		['-shop', false],
		['my shop', false]
	])('laravelNameIsValid(%j) is %j', (name, valid) => expect(laravelNameIsValid(name)).toBe(valid));

	it('compareMinor orders PHP minors numerically', () => {
		expect(compareMinor('8.10', '8.9')).toBeGreaterThan(0);
		expect(compareMinor('8.2', '8.2')).toBe(0);
		expect(compareMinor('7.4', '8.0')).toBeLessThan(0);
	});
});

describe('CreateProjectFlow form', () => {
	it('derives the folder name from the repository URL until the user types one', async () => {
		const flow = (await screen()).flow();
		flow.setSource('git');

		flow.repoUrl = 'https://github.com/necos98/lampo.git';
		flow.repoUrlEdited();
		expect(flow.folderName).toBe('lampo');

		flow.folderName = 'custom';
		flow.folderNameEdited();
		flow.repoUrl = 'https://github.com/necos98/other.git';
		flow.repoUrlEdited();
		expect(flow.folderName).toBe('custom');
	});

	it('needs a parent folder, a valid name and a URL (git) or a supported PHP (Laravel) to start', async () => {
		const flow = (await screen()).flow();
		expect(flow.canStart).toBe(false); // existing folder: nothing to scaffold

		flow.setSource('git');
		flow.parentDir = PARENT;
		flow.folderName = 'site';
		expect(flow.canStart).toBe(false);
		flow.repoUrl = 'https://example.com/site.git';
		expect(flow.canStart).toBe(true);
		flow.folderName = 'a:b';
		expect(flow.canStart).toBe(false);

		flow.setSource('laravel');
		flow.folderName = 'shop';
		expect(flow.canStart).toBe(true);
		flow.laravelPhp = '8.1';
		expect(flow.canStart).toBe(false);
		flow.laravelPhp = '8.10';
		expect(flow.canStart).toBe(true);
		flow.folderName = 'my shop';
		expect(flow.folderProblem).toBe('laravel-name');
		expect(flow.canStart).toBe(false);
	});

	it('builds the destination and the request from the trimmed form', async () => {
		const flow = (await screen()).flow();
		flow.setSource('git');
		flow.parentDir = ' C:\\Dev\\ ';
		flow.folderName = ' site ';
		flow.repoUrl = ' https://example.com/site.git ';
		flow.branch = '  ';

		expect(flow.destination).toBe('C:\\Dev\\site');
		expect(flow.request()).toEqual({
			parentDir: 'C:\\Dev\\',
			folderName: 'site',
			source: { kind: 'git-clone', url: 'https://example.com/site.git', branch: null }
		});
	});

	it('forgets the detection of another folder when the folder or the source changes', async () => {
		const { flow: makeFlow } = await screen();
		const flow = makeFlow();
		flow.folder = 'C:\\Dev\\shop2';
		const detecting = flow.detect();
		await settle();
		await detecting;
		expect(flow.detection).not.toBeNull();

		flow.folderEdited();

		expect(flow.detection).toBeNull();
		expect(flow.canCreate).toBe(false);
	});

	it('keeps the addressing the user picked over the default', async () => {
		const { env, projects, tasks, session } = await screen();
		let setting: Addressing | undefined;
		const flow = new CreateProjectFlow(env.backend, projects, testRunner(), tasks, session, () => setting);

		expect(flow.addressing).toBe('no-elevation'); // built-in fallback while settings are not loaded
		setting = 'standard';
		expect(flow.addressing).toBe('standard');
		flow.addressing = 'hosts-only';
		setting = 'no-elevation';
		expect(flow.addressing).toBe('hosts-only');
	});
});

describe('CreateProjectFlow detection', () => {
	it('fills preset, PHP and name from the detection', async () => {
		const flow = (await screen()).flow();
		flow.folder = ' C:\\Dev\\wp-site ';

		const detecting = flow.detect();
		expect(flow.detecting).toBe(true);
		await settle();
		await detecting;

		expect(flow.detecting).toBe(false);
		expect(flow.presetId).toBe('wordpress');
		expect(flow.phpVersion).toBe('8.3');
		expect(flow.name).toBe('wp-site');
		expect(flow.canCreate).toBe(true);
	});

	it('does nothing for an empty folder', async () => {
		const flow = (await screen()).flow();

		await flow.detect();

		expect(flow.detecting).toBe(false);
		expect(flow.detection).toBeNull();
	});

	it('keeps the result of the latest folder when an older detection ends later', async () => {
		const flow = (await screen()).flow();
		flow.folder = 'C:\\Dev\\wp-old';
		const first = flow.detect();
		flow.folder = 'C:\\Dev\\symfony-new';
		const second = flow.detect();
		await settle();
		await Promise.all([first, second]);

		expect(flow.detection?.folder).toBe('C:\\Dev\\symfony-new');
		expect(flow.presetId).toBe('symfony');
	});

	it('shows the error and no detection when it fails', async () => {
		const flow = (
			await screen({
				detectProject: async () => {
					throw { code: 'not-found', message: 'no folder' };
				}
			})
		).flow();
		flow.folder = 'C:\\Dev\\gone';

		await flow.detect();

		expect(flow.detection).toBeNull();
		expect(flow.detectError?.message).toBe('no folder');
		expect(flow.detecting).toBe(false);
	});

	it('cannot create a folder that is already a project', async () => {
		const flow = (await screen()).flow();
		flow.folder = 'C:\\Dev\\shop';

		const detecting = flow.detect();
		await settle();
		await detecting;

		expect(flow.detection?.alreadyRegistered).toBe('shop');
		expect(flow.canCreate).toBe(false);
		expect(await flow.create()).toBeNull();
	});

	it('creates and registers the detected project', async () => {
		const flow = (await screen()).flow();
		flow.folder = 'C:\\Dev\\fresh-site';
		const detecting = flow.detect();
		await settle();
		await detecting;

		const creating = flow.create();
		await settle();
		const project = await creating;

		expect(project?.id).toBe('fresh-site');
		expect(flow.creating).toBe(false);
	});
});

describe('CreateProjectFlow steps of a new project', () => {
	it('runs the setup, then moves on to detection when the task succeeds', async () => {
		const { flow: makeFlow, env, tasks } = await screen();
		const flow = makeFlow();
		fillLaravel(flow);
		flow.laravelPhp = '8.4';

		await flow.startScaffold();
		expect(flow.phase).toBe('running');
		expect(flow.taskId).toBe(TASK_ID);
		expect(tasks.isRunning(`scaffold:c:\\dev\\shop2`)).toBe(true);
		expect(flow.canStart).toBe(false);

		env.progress({ taskId: TASK_ID, stage: 'running', done: 0, total: 1, subject: 'laravel new', message: 'Creating project' });
		expect(flow.task?.lines).toEqual(['Creating project']);

		env.finish(TASK_ID, { status: 'succeeded' });
		await settle();

		expect(flow.phase).toBe('done');
		expect(flow.folder).toBe('C:\\Dev\\shop2');
		expect(flow.detection?.folder).toBe('C:\\Dev\\shop2');
		expect(flow.phpVersion).toBe('8.4'); // the PHP the installer ran with
	});

	it('shows the failure and keeps the values so the form can be edited', async () => {
		const { flow: makeFlow, env } = await screen();
		const flow = makeFlow();
		fillLaravel(flow);
		await flow.startScaffold();
		const error = { code: 'process-failed', message: 'composer failed' } as const;

		env.finish(TASK_ID, { status: 'failed', error });

		expect(flow.phase).toBe('failed');
		expect(flow.scaffoldError).toEqual(error);
		flow.editScaffold();
		expect(flow.phase).toBe('form');
		expect(flow.folderName).toBe('shop2');
	});

	it('goes back to the form, flagged as cancelled, when the task is cancelled', async () => {
		const { flow: makeFlow, env } = await screen();
		const flow = makeFlow();
		fillLaravel(flow);
		await flow.startScaffold();

		env.finish(TASK_ID, { status: 'cancelled' });

		expect(flow.phase).toBe('form');
		expect(flow.cancelled).toBe(true);
	});

	it('stays on the form with the error when the task cannot be started', async () => {
		const flow = (
			await screen({
				scaffoldProject: async () => {
					throw { code: 'package-missing', message: 'Git not found' };
				}
			})
		).flow();
		fillLaravel(flow);

		await flow.startScaffold();

		expect(flow.phase).toBe('form');
		expect(flow.scaffoldError?.message).toBe('Git not found');
		expect(flow.taskId).toBeNull();
		expect(flow.starting).toBe(false);
	});

	it('does not start twice, nor change source, while running', async () => {
		const scaffold = vi.fn(async () => ({ taskId: TASK_ID }));
		const flow = (await screen({ scaffoldProject: scaffold })).flow();
		fillLaravel(flow);

		await flow.startScaffold();
		await flow.startScaffold();
		flow.setSource('git');

		expect(scaffold).toHaveBeenCalledTimes(1);
		expect(flow.source).toBe('laravel');
	});

	it('ignores the end of a task that is not its own', async () => {
		const { flow: makeFlow, env } = await screen();
		const flow = makeFlow();
		fillLaravel(flow);
		await flow.startScaffold();

		env.finish('other-task', { status: 'succeeded' });

		expect(flow.phase).toBe('running');
	});

	it('cancels only while running', async () => {
		const cancelTask = vi.fn(async () => true);
		const { flow: makeFlow } = await screen({ cancelTask });
		const flow = makeFlow();
		fillLaravel(flow);

		await flow.cancelScaffold();
		expect(cancelTask).not.toHaveBeenCalled();

		await flow.startScaffold();
		await flow.cancelScaffold();
		expect(cancelTask).toHaveBeenCalledWith(TASK_ID);
	});
});

describe('CreateProjectFlow «Riprova» of a setup that did not start', () => {
	/** A screen whose first start is refused and the next ones accepted. */
	async function refusedOnce() {
		let calls = 0;
		const s = await screen({
			scaffoldProject: async () => {
				calls++;
				if (calls === 1) throw { code: 'process-failed', message: 'composer failed' };
				return { taskId: TASK_ID };
			}
		});
		return { ...s, calls: () => calls };
	}

	it('starts the setup again while the screen is shown', async () => {
		const { flow: makeFlow, notifier, session, calls } = await refusedOnce();
		const flow = makeFlow();
		fillLaravel(flow);
		await flow.startScaffold();
		expect(flow.scaffoldError?.code).toBe('process-failed');

		notifier.errorNotices[0]?.action?.run();
		await settle();

		expect(calls()).toBe(2);
		expect(flow.phase).toBe('running');
		expect(session.request?.folderName).toBe('shop2');
	});

	it('does nothing once the screen was left', async () => {
		const { flow: makeFlow, env, notifier, session, readyElsewhere, calls } = await refusedOnce();
		const flow = makeFlow();
		fillLaravel(flow);
		await flow.startScaffold();
		flow.dispose();

		notifier.errorNotices[0]?.action?.run();
		await settle();
		env.finish(TASK_ID, { status: 'succeeded' });
		await settle();

		expect(calls()).toBe(1);
		expect(session.request).toBeNull();
		expect(readyElsewhere).not.toHaveBeenCalled();
		expect(flow.phase).toBe('form');
	});
});

describe('CreateProjectFlow resume', () => {
	it('takes back a running setup, with the form values and its log, in a new visit', async () => {
		const { flow: makeFlow, env } = await screen();
		const first = makeFlow();
		first.setSource('git');
		first.parentDir = PARENT;
		first.repoUrl = 'https://example.com/site.git';
		first.repoUrlEdited();
		first.branch = 'develop';
		await first.startScaffold();
		env.progress({ taskId: TASK_ID, stage: 'running', done: 0, total: 1, subject: 'git clone', message: 'Cloning' });
		first.dispose();

		const second = makeFlow();
		second.resume();

		expect(second.phase).toBe('running');
		expect(second.source).toBe('git');
		expect(second.parentDir).toBe(PARENT);
		expect(second.folderName).toBe('site');
		expect(second.repoUrl).toBe('https://example.com/site.git');
		expect(second.branch).toBe('develop');
		expect(second.taskId).toBe(TASK_ID);
		expect(second.task?.lines).toEqual(['Cloning']);
	});

	it('resumes at the confirm step when the setup finished while away', async () => {
		const { flow: makeFlow, env, readyElsewhere } = await screen();
		const first = makeFlow();
		fillLaravel(first);
		first.laravelPhp = '8.4';
		await first.startScaffold();
		first.dispose();
		env.finish(TASK_ID, { status: 'succeeded' });
		expect(readyElsewhere).toHaveBeenCalledTimes(1);

		const second = makeFlow();
		second.resume();
		await settle();

		expect(second.phase).toBe('done');
		expect(second.detection?.folder).toBe('C:\\Dev\\shop2');
		expect(second.phpVersion).toBe('8.4');
	});

	it('does not announce the end to a screen that is on display', async () => {
		const { flow: makeFlow, env, readyElsewhere } = await screen();
		const flow = makeFlow();
		flow.resume();
		fillLaravel(flow);
		await flow.startScaffold();

		env.finish(TASK_ID, { status: 'succeeded' });

		expect(readyElsewhere).not.toHaveBeenCalled();
	});

	it('shows a failed setup of an earlier visit', async () => {
		const { flow: makeFlow, env } = await screen();
		const first = makeFlow();
		fillLaravel(first);
		await first.startScaffold();
		first.dispose();
		env.finish(TASK_ID, { status: 'failed', error: { code: 'process-failed', message: 'boom' } });

		const second = makeFlow();
		second.resume();

		expect(second.phase).toBe('failed');
		expect(second.scaffoldError?.message).toBe('boom');
	});

	it('starts from a clean form once the folder was registered', async () => {
		const { flow: makeFlow, env, session } = await screen();
		const first = makeFlow();
		fillLaravel(first);
		await first.startScaffold();
		env.finish(TASK_ID, { status: 'succeeded' });
		await settle();
		const creating = first.create();
		await settle();
		await creating;
		first.dispose();

		const second = makeFlow();
		second.resume();

		expect(session.request).toBeNull();
		expect(second.phase).toBe('form');
		expect(second.folderName).toBe('');
	});

	it('preselects the choices of the last creation, the parent folder only if it still exists', async () => {
		stored[SCAFFOLD_CHOICES_KEY] = JSON.stringify({ parentDir: PARENT, starterKit: 'livewire', testing: 'phpunit' });
		const flow = (await screen()).flow();

		flow.resume();
		await settle();

		expect(flow.starterKit).toBe('livewire');
		expect(flow.testing).toBe('phpunit');
		expect(flow.parentDir).toBe(PARENT);
	});

	it('leaves the parent folder empty when it is gone or the check fails', async () => {
		stored[SCAFFOLD_CHOICES_KEY] = JSON.stringify({ parentDir: 'C:\\missing', starterKit: 'vue', testing: 'pest' });
		const gone = (await screen()).flow();
		gone.resume();
		await settle();
		expect(gone.parentDir).toBe('');
		expect(gone.starterKit).toBe('vue');

		const failing = (
			await screen({
				scaffoldParentExists: async () => {
					throw { code: 'internal', message: 'boom' };
				}
			})
		).flow();
		failing.resume();
		await settle();
		expect(failing.parentDir).toBe('');
	});

	it('does not overwrite a parent folder typed while the check ran', async () => {
		stored[SCAFFOLD_CHOICES_KEY] = JSON.stringify({ parentDir: PARENT, starterKit: 'none', testing: 'pest' });
		const flow = (await screen()).flow();

		flow.resume();
		flow.parentDir = 'D:\\Typed';
		await settle();

		expect(flow.parentDir).toBe('D:\\Typed');
	});

	it('remembers the choices after creating a scaffolded project', async () => {
		const { flow: makeFlow, env } = await screen();
		const flow = makeFlow();
		fillLaravel(flow);
		flow.starterKit = 'react';
		flow.testing = 'phpunit';
		await flow.startScaffold();
		env.finish(TASK_ID, { status: 'succeeded' });
		await settle();

		const creating = flow.create();
		await settle();
		await creating;

		expect(JSON.parse(stored[SCAFFOLD_CHOICES_KEY])).toEqual({ parentDir: PARENT, starterKit: 'react', testing: 'phpunit' });
	});
});
