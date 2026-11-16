import {
	toAppError,
	type AppError,
	type Addressing,
	type Detection,
	type Project,
	type ScaffoldProjectRequest,
	type StarterKit,
	type TaskOutcome,
	type TestingFramework
} from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { effectiveAddressing } from '../projects/addressing.js';
import type { ProjectsStore } from './projects.svelte.js';
import { scaffoldViewOf, type ScaffoldPhase, type ScaffoldSession } from './scaffoldSession.js';
import { scaffoldTaskKey, type TaskState, type TasksStore } from './tasks.svelte.js';

const GENERIC_PRESET = 'generic-php';

/** Oldest PHP the Laravel Installer runs on (`laravel/installer` requires `^8.2`). */
export const LARAVEL_MIN_PHP = '8.2';

/** Orders PHP minors (`"8.10"` after `"8.9"`). */
export function compareMinor(a: string, b: string): number {
	const [am, an] = a.split('.').map(Number);
	const [bm, bn] = b.split('.').map(Number);
	return am - bm || an - bn;
}

/** Where the project comes from: a folder that already exists, a git clone or a new Laravel app. */
export type ProjectSource = 'existing' | 'git' | 'laravel';

export type { ScaffoldPhase };

/** Same rule as the backend (`folder_name_from_repo_url`): the last path segment without `.git`. */
export function folderNameFromRepoUrl(url: string): string {
	const last = url.trim().replace(/[\\/]+$/, '').split(/[\\/:]/).pop() ?? '';
	return last.replace(/\.git$/i, '');
}

const FORBIDDEN = /[<>:"/\\|?*\u0000-\u001f]/;
const RESERVED = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i;

/** Windows folder-name check (mirrors `FolderName::new`), `null` when valid. */
export function folderNameProblem(name: string): 'empty' | 'invalid' | null {
	const n = name.trim();
	if (!n) return 'empty';
	if (n === '.' || n === '..' || FORBIDDEN.test(n) || n.endsWith('.') || RESERVED.test(n.split('.')[0].trimEnd())) return 'invalid';
	return null;
}

/** The Laravel Installer's name rule: letters, digits, `-`, `_`, `.`, not starting with `-`. */
export function laravelNameIsValid(name: string): boolean {
	return /^[\p{L}\p{N}_.][\p{L}\p{N}_.-]*$/u.test(name.trim());
}

/**
 * The "add a project" flow. For an existing folder: folder -> detection suggests preset and PHP
 * -> user confirms. For a new project the folder is first created by a background task (git
 * clone or the Laravel Installer, with a live log), then the same detection/confirm step runs on
 * it. One instance per visit of the create screen; a setup started in an earlier visit and not
 * registered yet is picked up again through the `ScaffoldSession`.
 */
export class CreateProjectFlow {
	source = $state<ProjectSource>('existing');

	folder = $state('');
	detection = $state<Detection | null>(null);
	detecting = $state(false);
	detectError = $state<AppError | null>(null);

	presetId = $state(GENERIC_PRESET);
	phpVersion = $state<string | null>(null);
	name = $state('');
	creating = $state(false);

	// --- new folder (git / Laravel) ---
	parentDir = $state('');
	folderName = $state('');
	repoUrl = $state('');
	branch = $state('');
	laravelPhp = $state<string | null>(null);
	starterKit = $state<StarterKit>('none');
	testing = $state<TestingFramework>('pest');
	initGit = $state(false);
	npm = $state(false);

	phase = $state<ScaffoldPhase>('form');
	starting = $state(false);
	taskId = $state<string | null>(null);
	scaffoldError = $state<AppError | null>(null);
	/** The last run was cancelled by the user. */
	cancelled = $state(false);

	readonly #backend: Backend;
	readonly #projects: ProjectsStore;
	readonly #tasks: TasksStore;
	readonly #session: ScaffoldSession;
	readonly #defaultAddressing: () => Addressing | undefined;
	/** The mode picked on this screen; `null` follows the global default. */
	#chosenAddressing = $state<Addressing | null>(null);
	#generation = 0;
	/** The folder name follows the repository URL until the user types one. */
	#folderNameTouched = false;
	#stopListening: (() => void) | null = null;
	#detach: (() => void) | null = null;

	/** `defaultAddressing` reads the `default_addressing` setting (`undefined` while not loaded). */
	constructor(backend: Backend, projects: ProjectsStore, tasks: TasksStore, session: ScaffoldSession, defaultAddressing: () => Addressing | undefined) {
		this.#backend = backend;
		this.#projects = projects;
		this.#tasks = tasks;
		this.#session = session;
		this.#defaultAddressing = defaultAddressing;
	}

	/** Same rule as the CLI: the user's choice, else the `default_addressing` setting. Settings
	 * that load (or change) later still preselect, until the user picks a mode. */
	get addressing(): Addressing {
		return effectiveAddressing(this.#chosenAddressing, this.#defaultAddressing());
	}

	set addressing(mode: Addressing) {
		this.#chosenAddressing = mode;
	}

	get canCreate(): boolean {
		return this.detection !== null && !this.detection.alreadyRegistered && this.name.trim().length > 0 && !this.creating;
	}

	/** The running (or last) setup task. */
	get task(): TaskState | null {
		return this.taskId ? (this.#tasks.items[this.taskId] ?? null) : null;
	}

	/** `<parent>\<folder name>`, as the backend will create it. */
	get destination(): string {
		const parent = this.parentDir.trim().replace(/[\\/]+$/, '');
		const name = this.folderName.trim();
		return parent && name ? `${parent}\\${name}` : '';
	}

	get folderProblem(): 'empty' | 'invalid' | 'laravel-name' | null {
		const problem = folderNameProblem(this.folderName);
		if (problem) return problem;
		return this.source === 'laravel' && !laravelNameIsValid(this.folderName) ? 'laravel-name' : null;
	}

	get canStart(): boolean {
		if (this.source === 'existing' || this.starting || this.phase === 'running') return false;
		if (!this.parentDir.trim() || this.folderProblem) return false;
		return this.source === 'git' ? this.repoUrl.trim().length > 0 : this.laravelPhpSupported;
	}

	/** The Laravel Installer runs on PHP 8.2+ (`null` = the backend picks the newest installed). */
	get laravelPhpSupported(): boolean {
		return this.laravelPhp === null || compareMinor(this.laravelPhp, LARAVEL_MIN_PHP) >= 0;
	}

	/** Switching source keeps what was typed but forgets the detection of another folder. */
	setSource(source: ProjectSource): void {
		if (this.phase === 'running' || source === this.source) return;
		this.source = source;
		this.phase = 'form';
		this.scaffoldError = null;
		if (source !== 'existing') this.folderEdited();
		if (source === 'git' && !this.#folderNameTouched) this.folderName = folderNameFromRepoUrl(this.repoUrl);
	}

	repoUrlEdited(): void {
		if (!this.#folderNameTouched) this.folderName = folderNameFromRepoUrl(this.repoUrl);
	}

	folderNameEdited(): void {
		this.#folderNameTouched = this.folderName.trim().length > 0;
	}

	request(): ScaffoldProjectRequest {
		const base = { parentDir: this.parentDir.trim(), folderName: this.folderName.trim() };
		if (this.source === 'git') return { ...base, source: { kind: 'git-clone', url: this.repoUrl.trim(), branch: this.branch.trim() || null } };
		return {
			...base,
			source: { kind: 'laravel', phpVersion: this.laravelPhp, starterKit: this.starterKit, testing: this.testing, git: this.initGit, npm: this.npm }
		};
	}

	/** Starts the setup task; its end continues into detection (success) or the failure view. */
	async startScaffold(): Promise<void> {
		if (!this.canStart) return;
		const request = this.request();
		this.starting = true;
		this.scaffoldError = null;
		this.cancelled = false;
		this.#listen();
		try {
			const { taskId } = await this.#backend.commands.scaffoldProject(request);
			this.taskId = taskId;
			this.phase = 'running';
			this.#session.began(request, taskId);
			this.#tasks.track(taskId, { kind: 'scaffold', key: scaffoldTaskKey(request) });
		} catch (e) {
			this.scaffoldError = toAppError(e);
			this.phase = 'form';
		} finally {
			this.starting = false;
		}
	}

	async cancelScaffold(): Promise<void> {
		if (this.taskId && this.phase === 'running') await this.#tasks.cancel(this.taskId);
	}

	/** Back to the form after a failure or a cancel (values kept). */
	editScaffold(): void {
		if (this.phase === 'running') return;
		this.phase = 'form';
	}

	/**
	 * Call when the screen is shown: takes back the setup started in an earlier visit (running,
	 * or finished and not registered yet) with its form values, log and the step it reached.
	 */
	resume(): void {
		this.#detach ??= this.#session.attach();
		const request = this.#session.request;
		const task = request ? this.#tasks.forKey(scaffoldTaskKey(request)) : null;
		if (!request || !task) return;
		this.#restore(request);
		this.taskId = task.taskId;
		this.#listen();
		this.#apply(task.outcome);
	}

	/** Puts the values of a started setup back in the form. */
	#restore(request: ScaffoldProjectRequest): void {
		this.parentDir = request.parentDir;
		this.folderName = request.folderName ?? '';
		this.#folderNameTouched = true;
		const source = request.source;
		if (source.kind === 'git-clone') {
			this.source = 'git';
			this.repoUrl = source.url;
			this.branch = source.branch ?? '';
		} else {
			this.source = 'laravel';
			this.laravelPhp = source.phpVersion ?? null;
			this.starterKit = source.starterKit ?? this.starterKit;
			this.testing = source.testing ?? this.testing;
			this.initGit = source.git ?? false;
			this.npm = source.npm ?? false;
		}
	}

	#listen(): void {
		this.#stopListening ??= this.#tasks.onFinished((t) => this.#finished(t));
	}

	#finished(task: TaskState): void {
		if (task.taskId !== this.taskId || !task.outcome) return;
		this.#apply(task.outcome);
	}

	/** Shows the step a setup task reached; a finished one continues into detection. */
	#apply(outcome: TaskOutcome | null): void {
		const view = scaffoldViewOf(outcome);
		this.phase = view.phase;
		this.cancelled = view.cancelled;
		this.scaffoldError = view.error;
		if (view.phase !== 'done') return;
		this.folder = this.destination;
		const php = this.source === 'laravel' ? this.laravelPhp : null;
		void this.detect().then(() => {
			// The installer ran with this PHP: the natural default for the project too.
			if (php && this.detection) this.phpVersion = php;
		});
	}

	/** Call when leaving the screen: stops following the task and lets the session announce its end. */
	dispose(): void {
		this.#stopListening?.();
		this.#stopListening = null;
		this.#detach?.();
		this.#detach = null;
	}

	async detect(): Promise<void> {
		const folder = this.folder.trim();
		if (!folder) return;
		const generation = ++this.#generation;
		this.detecting = true;
		this.detectError = null;
		try {
			const d = await this.#backend.commands.detectProject(folder);
			if (generation !== this.#generation) return;
			this.detection = d;
			this.presetId = d.presetId ?? GENERIC_PRESET;
			this.phpVersion = d.phpVersion;
			this.name = d.suggestedName;
		} catch (e) {
			if (generation !== this.#generation) return;
			this.detection = null;
			this.detectError = toAppError(e);
		} finally {
			if (generation === this.#generation) this.detecting = false;
		}
	}

	/** Typing a different folder invalidates the previous detection. */
	folderEdited(): void {
		this.#generation++;
		this.detection = null;
		this.detectError = null;
		this.detecting = false;
	}

	async create(): Promise<Project | null> {
		if (!this.detection || !this.canCreate) return null;
		this.creating = true;
		const result = await this.#projects.create({
			root: this.detection.folder,
			name: this.name.trim(),
			id: this.detection.suggestedId,
			presetId: this.presetId,
			phpVersion: this.phpVersion,
			addressing: this.addressing,
			activate: true
		});
		this.creating = false;
		if (!result.ok) return null;
		// The scaffolded folder is now a project: a later visit starts from a clean form.
		if (this.source !== 'existing') this.#session.registered();
		return result.value;
	}
}
