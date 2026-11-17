/**
 * Simulated backend for `pnpm dev:mock`: realistic data, simulated task progress, service
 * transitions and the failure modes the GUI has to handle (port in use, network failure,
 * elevation required). State lives in memory and resets on reload.
 */
import type { AppError } from '../../api/types.js';
import type * as T from '../../api/types.js';
import type { Backend, Commands, Platform, Updater } from '../ports.js';
import { MockEmitter } from './emitter.js';
import { checkEnvVars } from './envVars.js';
import { MockLogs } from './logs.js';
import { connection, mysqlOf, packageKey, seedCatalog, seedPresets, seedProjects, seedWorktrees, seriesOf } from './seed.js';

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
// JSON round trip like Tauri IPC: `structuredClone` rejects the `$state` proxies the UI passes as arguments.
const clone = <V>(v: V): V => (v === undefined ? v : (JSON.parse(JSON.stringify(v)) as V));

function fail(code: AppError['code'], message: string, details?: unknown): never {
	throw { code, message, details } satisfies AppError;
}

/** An absolute Windows folder; one whose path contains "missing" simulates a deleted folder. */
function folderExists(path: string): boolean {
	return /^[a-z]:\\/i.test(path.trim()) && !/missing/i.test(path);
}

function urlsFor(addresses: T.SiteAddress[]): string[] {
	return addresses.map((a) => (a.kind === 'loopback-port' ? `http://127.0.0.1:${a.port}` : `http://${a.host}`));
}

interface Options {
	/** Start in the first-run state (nothing installed). */
	fresh: boolean;
}

export function createMockBackend({ fresh }: Options): Backend {
	const emitter = new MockEmitter();
	const logs = new MockLogs();
	const cancelled = new Set<string>();
	let taskSeq = 0;
	let firstRunFailures = 1;
	let port5Failures = 1;
	let hostsInSync = true;
	let firstRunCompleted = !fresh;

	let projects = seedProjects();
	let worktrees = seedWorktrees();
	const presets = seedPresets();
	const { catalog, installed: seededInstalled } = seedCatalog();
	let installed: T.PackageId[] = fresh ? [] : seededInstalled;
	let settings: T.Settings = {
		defaultPhpVersion: '8.4',
		tld: 'test',
		apachePort: 80,
		services: [
			{ service: 'mysql-8.4', label: 'MySQL 8.4', port: 3306, alwaysOn: true },
			{ service: 'mysql-5.7', label: 'MySQL 5.7', port: 3307, alwaysOn: false },
			{ service: 'redis', label: 'Redis', port: 6379, alwaysOn: false },
			{ service: 'mailpit', label: 'Mailpit', port: 1025, alwaysOn: false, uiPort: 8025 }
		],
		loopbackPortRange: { start: 8100, end: 8199 },
		closeToTray: true,
		autoRestartServices: true,
		mysqlProjectCredentials: true,
		firstRunCompleted: !fresh,
		defaultAddressing: 'no-elevation'
	};
	let shims: T.ShimStatus = { shimDir: 'C:\\Users\\dev\\AppData\\Local\\Lampo\\shims', installed: true, onPath: false, shadowed: [{ tool: 'php', shim: 'C:\\Users\\dev\\AppData\\Local\\Lampo\\shims\\php.exe', shadowedBy: 'C:\\xampp\\php\\php.exe' }] };
	const expectedPlugin = '0.1.0-mock-h1a2b3c4d';
	let claudeCode: T.ClaudeCodeStatus = {
		state: 'not-installed',
		claudeVersion: '2.1.0 (Claude Code)',
		installedVersion: null,
		expectedVersion: expectedPlugin,
		pluginDir: 'C:\\Users\\dev\\AppData\\Local\\Lampo\\agent-plugins\\claude-code',
		cliPath: 'C:\\Users\\dev\\AppData\\Local\\Lampo\\bin\\lampo.exe',
		cliInstalled: true,
		updateError: null
	};
	const step = (s: string, detail: string | null = null): T.ClaudeCodeStep => ({ step: s, outcome: 'ok', detail });

	const services = new Map<string, T.ServiceStatus>();
	const serviceLabel = (service: string) => (service.startsWith('mysql-') ? `MySQL ${service.slice(6)}` : service.charAt(0).toUpperCase() + service.slice(1));
	const addService = (s: Partial<T.ServiceStatus> & Pick<T.ServiceStatus, 'service' | 'kind'>) =>
		services.set(s.service, { label: serviceLabel(s.service), state: 'stopped', pid: null, port: null, message: null, required: false, requiredBy: [], ...s });
	addService({ service: 'apache', kind: 'apache', state: fresh ? 'stopped' : 'running', pid: fresh ? null : 4120, port: 80, required: true });
	addService({ service: 'mysql-8.4', kind: 'mysql', state: fresh ? 'stopped' : 'running', pid: fresh ? null : 5230, port: 3306, required: true });
	addService({ service: 'mysql-5.7', kind: 'mysql', port: 3307, requiredBy: ['crm-legacy'] });
	addService({ service: 'redis', kind: 'redis', state: fresh ? 'stopped' : 'running', pid: fresh ? null : 6310, port: 6379, requiredBy: ['shop'] });
	addService({ service: 'mailpit', kind: 'mailpit', state: fresh ? 'stopped' : 'running', pid: fresh ? null : 7020, port: 1025, uiPort: 8025, uiUrl: 'http://127.0.0.1:8025', requiredBy: ['shop'] });

	const syncProject = (id: string) => emitter.emit<T.ProjectsChangedPayload>('projects-changed', { projectIds: [id] });
	const syncWorktrees = (projectId: string) => emitter.emit<T.WorktreesChangedPayload>('worktrees-changed', { projectId });
	const syncPackages = () => emitter.emit<void>('packages-changed', undefined as never);

	const isInstalled = (p: T.PackageId) => installed.some((i) => packageKey(i) === packageKey(p));
	const withInstalledFlag = (e: T.CatalogEntry): T.CatalogEntry => ({ ...e, installed: isInstalled(e.id) });
	/** Like the backend: projects using Mailpit get its SMTP endpoint and web UI. */
	const mailOf = (p: T.Project): T.MailConnection | null => {
		if (!p.environment.services.some((s) => s.kind === 'mailpit')) return null;
		const instance = settings.services.find((s) => s.service === 'mailpit');
		return { host: '127.0.0.1', smtpPort: instance?.port ?? 1025, webUrl: `http://127.0.0.1:${instance?.uiPort ?? 8025}` };
	};
	const attachWorktrees = (p: T.Project): T.Project => ({ ...p, mail: mailOf(p), worktrees: worktrees.filter((w) => w.projectId === p.id) });

	const find = (id: string) => projects.find((p) => p.id === id) ?? fail('not-found', `project ${id}`);
	/** Like the app's `OpenProjectIn`: Explorer, the terminal and VS Code need the folder to exist. */
	const openIn = async (id: string) => {
		await sleep(150);
		const project = find(id);
		if (!project.folderExists) fail('validation', `the project folder does not exist: ${project.root}`, { folder: project.root });
	};
	const replace = (next: T.Project) => {
		projects = projects.map((p) => (p.id === next.id ? next : p));
		return attachWorktrees(clone(next));
	};
	/** Like the backend: PHP before 7.4 cannot authenticate to MySQL 8 with its default plugin. */
	const warningsOf = (e: T.ProjectEnvironment): T.ProjectWarning[] => {
		const mysql = mysqlOf(e);
		const [major, minor] = e.phpVersion.split('.').map(Number);
		const legacyPhp = major < 7 || (major === 7 && minor < 4);
		return mysql && Number(mysql.split('.')[0]) >= 8 && legacyPhp
			? [{ code: 'legacy-php-mysql-auth', message: `PHP ${e.phpVersion} cannot use MySQL ${mysql}'s default authentication (caching_sha2_password)` }]
			: [];
	};
	const rebuild = (p: T.Project): T.Project => ({
		...p,
		warnings: warningsOf(p.environment),
		url: urlsFor(p.addresses)[0] ?? '',
		urls: urlsFor(p.addresses),
		documentRoot: p.environment.publicDir ? `${p.root}\\${p.environment.publicDir}` : p.root,
		database: mysqlOf(p.environment) ? connection(p.databaseName ?? p.id.replace(/-/g, '_'), mysqlOf(p.environment) as string, p.mysqlCredentials === 'project' ? p.id : null) : null
	});
	const hasHostname = (p: T.Project) => p.addresses.some((a) => a.kind === 'hostname');
	/** Like the backend: domain default addresses and the lowest free loopback port of the range. */
	const suggest = (id: string): T.AddressSuggestions => {
		const used = new Set([...projects.flatMap((p) => p.addresses.flatMap((a) => (a.kind === 'loopback-port' ? [a.port] : []))), ...worktrees.map((w) => w.loopbackPort)]);
		let port = settings.loopbackPortRange.start;
		while (used.has(port)) port++;
		const hostname: T.SiteAddress = { kind: 'hostname', host: `${id}.${settings.tld}` };
		const localhost: T.SiteAddress = { kind: 'localhost', host: `${id}.localhost` };
		const loopbackPort: T.SiteAddress = { kind: 'loopback-port', port };
		return { hostname, localhost, loopbackPort, noElevation: [localhost, loopbackPort], standard: [hostname, localhost], hostsOnly: [hostname], defaultAddressing: settings.defaultAddressing };
	};

	// ---- tasks ---------------------------------------------------------------------------------

	/** Unfinished tasks, for `listRunningTasks` (a recreated window re-attaches to them). */
	const running = new Map<string, T.RunningTask>();

	/** Simulates a background task: progress events, then a finished event. */
	function startTask(
		operation: T.TaskOperation,
		subjects: string[],
		opts: { failAt?: { subject: string; error: AppError }; onSubjectDone?: (subject: string) => void; onSuccess?: () => void } = {}
	): T.TaskStarted {
		const taskId = `task-${++taskSeq}`;
		running.set(taskId, { taskId, operation: clone(operation), progress: null, subjects: [] });
		void (async () => {
			await sleep(60);
			for (const subject of subjects) {
				const stages: T.ProgressStage[] = ['resolving', 'downloading', 'verifying', 'extracting', 'configuring'];
				for (const stage of stages) {
					const steps = stage === 'downloading' ? 12 : 1;
					for (let i = 1; i <= steps; i++) {
						if (cancelled.has(taskId)) return finish(taskId, { status: 'cancelled' });
						await sleep(stage === 'downloading' ? 120 : 140);
						const total = stage === 'downloading' ? 30_000_000 : null;
						const report: T.ProgressReport = { stage, done: total ? Math.round((i / steps) * total) : 0, total, subject };
						const task = running.get(taskId);
						if (task) {
							task.progress = report;
							task.subjects = [...task.subjects.filter((s) => s.subject !== subject), report];
						}
						emitter.emit<T.TaskProgressPayload>('task-progress', { taskId, ...report });
					}
				}
				if (opts.failAt?.subject === subject) return finish(taskId, { status: 'failed', error: opts.failAt.error });
				opts.onSubjectDone?.(subject);
			}
			opts.onSuccess?.();
			finish(taskId, { status: 'succeeded' });
		})();
		return { taskId };
	}
	/** A task that runs external programs: one `running` progress event per output line. */
	function startLogTask(operation: T.TaskOperation, steps: { subject: string; lines: string[] }[], opts: { failWith?: AppError; onSuccess?: () => void } = {}): T.TaskStarted {
		const taskId = `task-${++taskSeq}`;
		running.set(taskId, { taskId, operation: clone(operation), progress: null, subjects: [] });
		void (async () => {
			await sleep(60);
			for (const [index, step] of steps.entries()) {
				for (const message of step.lines) {
					if (cancelled.has(taskId)) return finish(taskId, { status: 'cancelled' });
					await sleep(90);
					const report: T.ProgressReport = { stage: 'running', done: index, total: steps.length, subject: step.subject, message };
					const task = running.get(taskId);
					if (task) task.progress = report;
					emitter.emit<T.TaskProgressPayload>('task-progress', { taskId, ...report });
				}
			}
			if (opts.failWith) return finish(taskId, { status: 'failed', error: opts.failWith });
			opts.onSuccess?.();
			finish(taskId, { status: 'succeeded' });
		})();
		return { taskId };
	}
	function finish(taskId: string, outcome: T.TaskOutcome) {
		cancelled.delete(taskId);
		running.delete(taskId);
		emitter.emit<T.TaskFinishedPayload>('task-finished', { taskId, outcome });
	}

	const addPackage = (p: T.PackageId) => {
		if (!isInstalled(p)) installed = [...installed, p];
	};

	// ---- services ------------------------------------------------------------------------------

	function setService(name: string, patch: Partial<T.ServiceStatus>) {
		const cur = services.get(name) ?? fail('not-found', name);
		const next = { ...cur, ...patch };
		services.set(name, next);
		emitter.emit<T.ServiceStatusPayload>('service-status', { service: name, kind: next.kind, state: next.state as T.ServiceStatusPayload['state'], pid: next.pid, port: next.port, message: next.message });
	}
	const needed = (s: T.ServiceStatus) => s.required || s.requiredBy.length > 0;
	const targets = (req: T.ServicesRequest) => (req.services.length ? req.services : [...services.values()].filter(needed).map((s) => s.service));
	const report = (over: Partial<T.SyncReport> = {}): T.SyncReport => ({ started: [], stopped: [], restarted: [], hostsFileUpdated: false, warnings: [], ...over });

	async function startOne(name: string) {
		setService(name, { state: 'starting', message: null });
		await sleep(700);
		if (name === 'mysql-5.7' && port5Failures > 0) {
			port5Failures--;
			const details = { port: 3307, owner: 'mysqld.exe (PID 4412)' };
			setService(name, { state: 'failed', pid: null, message: 'port 3307 already in use' });
			fail('port-in-use', 'port 3307 already in use', details);
		}
		setService(name, { state: 'running', pid: 4000 + Math.floor(Math.random() * 3000), message: null });
	}
	async function stopOne(name: string) {
		setService(name, { state: 'stopping' });
		await sleep(500);
		setService(name, { state: 'stopped', pid: null });
	}

	const commands: Commands = {
		appInfo: async () => ({ version: '0.1.0-mock', configDir: 'C:\\Users\\dev\\AppData\\Roaming\\Lampo', dataDir: 'C:\\Users\\dev\\AppData\\Local\\Lampo', shimDir: shims.shimDir, controlProtocolVersion: 1 }),
		getStatus: async () => ({
			appRunning: true,
			services: [...services.values()].map(clone),
			projects: projects.map((p) => ({ id: p.id, name: p.name, root: p.root, active: p.active, url: p.url, phpVersion: p.environment.phpVersion, mysqlVersion: mysqlOf(p.environment), worktreeCount: worktrees.filter((w) => w.projectId === p.id).length })),
			worktreeCount: worktrees.length,
			unreadableProjects: []
		}),
		quitApp: async () => {},
		prepareUpdate: async () => {
			for (const s of [...services.values()]) if (s.state === 'running') await stopOne(s.service);
		},
		cancelUpdate: async () => {},

		// Projects
		listProjects: async () => {
			await sleep(250);
			return { projects: projects.map((p) => attachWorktrees(clone(p))), unreadable: [] };
		},
		getProject: async (id) => attachWorktrees(clone(find(id))),
		suggestAddresses: async (projectId) => {
			await sleep(60);
			return suggest(projectId);
		},
		detectProject: async (folder) => {
			await sleep(400);
			const name = folder.split(/[\\/]/).filter(Boolean).pop() ?? 'sito';
			const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, '-');
			const lower = folder.toLowerCase();
			const wp = lower.includes('wp') || lower.includes('blog') || lower.includes('wordpress');
			const sf = lower.includes('symfony');
			const generic = lower.includes('generic') || lower.includes('plain');
			const legacy = lower.includes('legacy') || lower.includes('old');
			const laravel = !wp && !sf && !generic && !legacy;
			return {
				folder,
				projectType: laravel ? 'laravel' : wp ? 'wordpress' : sf ? 'symfony' : legacy ? 'legacy' : null,
				presetId: laravel ? 'laravel' : wp ? 'wordpress' : sf ? 'symfony' : legacy ? 'legacy' : 'generic-php',
				phpVersion: legacy ? '7.4' : wp ? '8.3' : generic ? null : '8.4',
				suggestedName: name,
				suggestedId: slug,
				evidence: laravel ? ['artisan', 'composer.json: laravel/framework ^11.0', 'composer.json: php ^8.2'] : wp ? ['wp-config.php', 'wp-content/'] : sf ? ['symfony.lock', 'composer.json: symfony/framework-bundle'] : legacy ? ['index.php'] : [],
				services: laravel ? ['mysql'] : [],
				warnings: [],
				alreadyRegistered: projects.find((p) => p.root.toLowerCase() === lower)?.id ?? null
			} satisfies T.Detection;
		},
		createProject: async (r) => {
			await sleep(500);
			const preset = presets.find((p) => p.id === r.presetId) ?? presets.find((p) => p.id === 'generic-php')!;
			const name = r.name ?? r.root.split(/[\\/]/).filter(Boolean).pop() ?? 'sito';
			const id = r.id ?? name.toLowerCase().replace(/[^a-z0-9]+/g, '-');
			if (projects.some((p) => p.id === id)) fail('conflict', `project ${id} exists`);
			const environment: T.ProjectEnvironment = clone(r.environment ?? preset.environment);
			if (r.phpVersion) environment.phpVersion = r.phpVersion;
			const suggested = suggest(id);
			const byMode: Record<T.Addressing, T.SiteAddress[]> = { standard: suggested.standard, 'hosts-only': suggested.hostsOnly, 'no-elevation': suggested.noElevation };
			// Like the backend: no explicit mode means the `default_addressing` setting.
			const addresses = r.addresses ?? byMode[r.addressing ?? settings.defaultAddressing];
			const next = rebuild({ id, name, root: r.root, folderExists: true, addresses, url: '', urls: [], documentRoot: '', environment, databaseName: mysqlOf(environment) ? id.replace(/-/g, '_') : null, database: null, mysqlCredentials: settings.mysqlProjectCredentials ? 'project' : 'root', tls: 'disabled', active: r.activate ?? true, originPreset: preset.id, worktrees: [], notices: [], warnings: [] });
			projects = [...projects, next];
			if (hasHostname(next)) hostsInSync = false;
			syncProject(id);
			const result: T.CreateProjectResult = attachWorktrees(clone(next));
			if (next.databaseName && r.createDatabase !== false) {
				// A folder name containing "nodb" simulates a MySQL instance that does not start.
				result.databaseSetup = r.root.toLowerCase().includes('nodb')
					? { name: next.databaseName, status: 'failed', message: 'MySQL 8.4 did not start: port 3306 is in use' }
					: { name: next.databaseName, status: 'created', message: null };
			}
			return result;
		},
		scaffoldParentExists: async (parentDir) => {
			await sleep(40);
			return folderExists(parentDir);
		},
		scaffoldProject: async (r) => {
			await sleep(80);
			if (!r.parentDir.trim()) fail('validation', 'invalid new project: the parent folder is required');
			if (!folderExists(r.parentDir)) fail('validation', `the folder \`${r.parentDir.trim()}\` does not exist`);
			const source = r.source;
			const folderName = r.folderName?.trim() || (source.kind === 'git-clone' ? source.url.replace(/[\\/]+$/, '').split(/[\\/:]/).pop()?.replace(/\.git$/, '') : '') || '';
			if (!folderName) fail('validation', 'invalid new project: the folder name is empty');
			const dest = `${r.parentDir.replace(/[\\/]+$/, '')}\\${folderName}`;
			if (projects.some((p) => p.root.toLowerCase() === dest.toLowerCase())) fail('validation', `invalid new project: the folder \`${dest}\` already exists`);
			if (source.kind === 'git-clone') {
				if (!source.url.trim()) fail('validation', 'invalid new project: the repository URL is empty');
				// A URL containing "nogit" simulates a machine without git.
				if (source.url.includes('nogit')) fail('package-missing', 'Git not found: install it from git-scm.com', { tool: 'git' });
				const percent = [8, 23, 47, 69, 88, 100];
				const lines = [`> git clone`, `Cloning into '${dest}'...`, 'remote: Enumerating objects: 1342, done.', ...percent.map((p) => `Receiving objects: ${String(p).padStart(3)}% (${Math.round(13.42 * p)}/1342)`), 'Resolving deltas: 100% (512/512), done.'];
				// A URL containing "fail" simulates a clone that fails after a few lines of output.
				const failing = source.url.includes('fail');
				const failedLines = lines.slice(0, 3).concat("fatal: repository 'x' not found");
				// Like the backend: `details.output` is the program's last lines as one string ("> git clone" is Lampo's own line).
				const failWith = failing ? ({ code: 'process-failed', message: "git clone failed (exit code 128): fatal: repository 'x' not found", details: { output: failedLines.slice(1).join('\n') } } satisfies AppError) : undefined;
				return startLogTask({ kind: 'scaffold-project', request: r }, [{ subject: 'git clone', lines: failing ? failedLines : lines }], { failWith });
			}
			const php = source.phpVersion ?? '8.4';
			if (!installed.some((i) => i.kind === 'php' && i.version.startsWith(`${php}.`))) fail('package-missing', `PHP ${php} is not installed`);
			if (!installed.some((i) => i.kind === 'composer')) fail('package-missing', 'Composer is not installed');
			const kit = source.starterKit && source.starterKit !== 'none' ? `laravel/${source.starterKit}-starter-kit` : 'laravel/laravel';
			return startLogTask({ kind: 'scaffold-project', request: r }, [
				{ subject: 'laravel-installer', lines: ['> composer update (Laravel Installer)', 'Loading composer repositories with package information', 'Updating dependencies', 'Nothing to modify in lock file', 'Generating autoload files'] },
				{
					subject: 'laravel new',
					lines: [
						'> laravel new',
						`Creating a "${kit}" project at "./${folderName}"`,
						'Installing laravel/laravel (v12.1.0)',
						'  - Downloading laravel/laravel (v12.1.0)',
						'Updating dependencies',
						'Lock file operations: 112 installs, 0 updates, 0 removals',
						'Generating optimized autoload files',
						'   INFO  Application key set successfully.',
						...(source.testing === 'phpunit' ? [] : ['Installing Pest...']),
						...(source.git ? ['   INFO  Git repository initialized.'] : []),
						...(source.npm ? ['added 142 packages in 9s', 'vite v6.2.0 building for production...'] : []),
						'   INFO  Application ready in [' + folderName + "]. You can start your local development using:"
					]
				}
			]);
		},
		updateProject: async (r) => {
			await sleep(350);
			const cur = find(r.projectId);
			if (r.environment.apacheDirectives.includes('!!')) fail('config-rejected', 'httpd -t failed', { output: 'AH00526: Syntax error on line 3 of vhost.conf:\nInvalid command \'!!\', perhaps misspelled or defined by a module not included in the server configuration' });
			const refusedEnvVars = checkEnvVars(r.environment.envVars);
			if (refusedEnvVars) fail('validation', refusedEnvVars.message, refusedEnvVars.details);
			const next = rebuild({ ...cur, name: r.name, addresses: r.addresses, environment: r.environment, databaseName: r.databaseName, mysqlCredentials: r.mysqlCredentials ?? cur.mysqlCredentials, tls: r.tls });
			if (JSON.stringify(cur.addresses) !== JSON.stringify(next.addresses) && hasHostname(next)) hostsInSync = false;
			const out = replace(next);
			syncProject(next.id);
			return out;
		},
		deleteProject: async (id) => {
			await sleep(250);
			find(id);
			const orphanedDatabases = worktrees.flatMap((w) => (w.projectId === id && w.databaseMode.mode !== 'shared' && w.database ? [w.database.database] : []));
			projects = projects.filter((p) => p.id !== id);
			worktrees = worktrees.filter((w) => w.projectId !== id);
			syncProject(id);
			return { orphanedDatabases };
		},
		setProjectActive: async ({ projectId, active }) => {
			await sleep(300);
			const next = replace({ ...find(projectId), active });
			syncProject(projectId);
			if (active && hasHostname(next) && !hostsInSync) fail('elevation-required', 'hosts file needs admin rights');
			return next;
		},
		setProjectPhp: async ({ projectId, phpVersion }) => {
			await sleep(200);
			const cur = find(projectId);
			const next = replace(rebuild({ ...cur, environment: { ...cur.environment, phpVersion } }));
			syncProject(projectId);
			return next;
		},
		relinkProject: async ({ projectId, root }) => {
			await sleep(300);
			const next = replace(rebuild({ ...find(projectId), root, folderExists: true }));
			syncProject(projectId);
			return next;
		},
		exportProject: async ({ projectId }) => {
			await sleep(250);
			find(projectId);
		},
		openTerminal: openIn,
		openProjectFolder: openIn,
		openInVsCode: openIn,
		importProject: async ({ file, root }) => {
			await sleep(400);
			const name = file.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, '') ?? 'importato';
			if (projects.some((p) => p.id === name)) fail('conflict', 'project exists');
			const base = projects[0];
			const next = rebuild({ ...clone(base), id: name, name, root: root ?? `C:\\Dev\\${name}`, addresses: [{ kind: 'localhost', host: `${name}.localhost` }], worktrees: [], originPreset: null });
			projects = [...projects, next];
			syncProject(name);
			// A file whose name says "shared" shows the review of what an import leaves out.
			const shared = file.toLowerCase().includes('shared');
			return {
				...attachWorktrees(clone(next)),
				droppedApacheDirectives: shared ? 'CustomLog "|C:/Windows/System32/cmd.exe /c calc" common' : null,
				droppedPhpIniKeys: shared ? ['auto_prepend_file'] : [],
				droppedEnvVars: shared ? ['COMSPEC'] : []
			};
		},

		// Presets
		listPresets: async () => presets.map(clone),
		savePreset: async ({ projectId, name, description }) => {
			await sleep(250);
			const p = { id: name.toLowerCase().replace(/[^a-z0-9]+/g, '-'), name, description: description ?? '', builtin: false, environment: clone(find(projectId).environment) };
			presets.push(p);
			return clone(p);
		},
		deletePreset: async (id) => {
			const i = presets.findIndex((p) => p.id === id && !p.builtin);
			if (i < 0) fail('not-found', id);
			presets.splice(i, 1);
		},

		// Packages
		listCatalog: async (kind = null) => {
			await sleep(300);
			// Ordered like the backend: by kind, then newest series, PHP minor and version first.
			const num = (a: string, b: string) => b.localeCompare(a, undefined, { numeric: true });
			const order = (a: T.CatalogEntry, b: T.CatalogEntry) => (a.id.kind < b.id.kind ? -1 : a.id.kind > b.id.kind ? 1 : num(a.series, b.series) || num(a.id.phpVersion ?? '', b.id.phpVersion ?? '') || num(a.id.version, b.id.version));
			return catalog.filter((e) => !kind || e.id.kind === kind).sort(order).map(withInstalledFlag).map(clone);
		},
		listInstalledPackages: async () => {
			await sleep(200);
			return installed.map((id) => ({
				id: clone(id),
				series: seriesOf(id.version),
				path: `C:\\Users\\dev\\AppData\\Local\\Lampo\\packages\\${packageKey(id)}`,
				usedBy: id.kind === 'php' ? projects.filter((p) => id.version.startsWith(`${p.environment.phpVersion}.`) && p.active).map((p) => p.id) : id.kind === 'mysql' ? (id.version.startsWith('8.4') ? ['system'] : projects.filter((p) => mysqlOf(p.environment) === '5.7').map((p) => p.id)) : ['system']
			}));
		},
		checkPackageUpdates: async () => {
			await sleep(700);
			return isInstalled({ kind: 'php', version: '8.3.12', phpVersion: null }) ? [{ kind: 'php', series: '8.3', phpVersion: null, installed: '8.3.12', latest: '8.3.14' }] : [];
		},
		installPackage: async (r) => {
			await sleep(80);
			const series = r.series ?? null;
			const wanted = catalog.filter((e) => e.id.kind === r.kind && (r.phpVersion ?? null) === e.id.phpVersion && (r.version ? e.id.version === r.version : series ? e.id.version.startsWith(`${series}.`) : true));
			const target = wanted.sort((a, b) => b.id.version.localeCompare(a.id.version, undefined, { numeric: true }))[0] ?? fail('not-found', `${r.kind} ${r.version ?? series ?? ''}`);
			const subject = packageKey(target.id);
			return startTask({ kind: 'install-package', request: r }, [subject], {
				onSuccess: () => {
					// An update replaces the previous patch of the same series.
					installed = installed.filter((i) => !(i.kind === target.id.kind && i.phpVersion === target.id.phpVersion && i.version.split('.').slice(0, 2).join('.') === target.id.version.split('.').slice(0, 2).join('.')));
					addPackage(target.id);
					syncPackages();
				}
			});
		},
		uninstallPackage: async (id) => {
			await sleep(80);
			const subject = packageKey(id);
			const steps = [{ subject, lines: ['Riallineamento di Apache e dei servizi', `Rimozione dei file di ${subject}`] }];
			return startLogTask({ kind: 'uninstall-package', id }, steps, {
				onSuccess: () => {
					installed = installed.filter((i) => packageKey(i) !== subject);
					syncPackages();
				}
			});
		},
		firstRunStatus: async () => {
			await sleep(150);
			const has = (kind: string, series?: string) => installed.some((i) => i.kind === kind && (!series || i.version.startsWith(`${series}.`)));
			return {
				completed: firstRunCompleted,
				items: [
					{ kind: 'apache', series: '2.4', installed: has('apache') },
					{ kind: 'mod-fcgid', series: null, installed: has('mod-fcgid') },
					{ kind: 'php', series: '8.4', installed: has('php', '8.4') },
					{ kind: 'mysql', series: '8.4', installed: has('mysql', '8.4') },
					{ kind: 'composer', series: null, installed: has('composer') }
				]
			};
		},
		runFirstRun: async () => {
			await sleep(100);
			const wanted = [catalog.find((e) => e.id.kind === 'apache')!, catalog.find((e) => e.id.kind === 'mod-fcgid')!, catalog.find((e) => e.id.kind === 'php' && e.id.version === '8.4.1')!, catalog.find((e) => e.id.kind === 'mysql' && e.id.version.startsWith('8.4'))!, catalog.find((e) => e.id.kind === 'composer')!].filter((e) => !isInstalled(e.id));
			const shouldFail = firstRunFailures > 0 && wanted.some((e) => e.id.kind === 'mysql');
			if (shouldFail) firstRunFailures--;
			return startTask({ kind: 'first-run' }, wanted.map((e) => packageKey(e.id)), {
				failAt: shouldFail ? { subject: packageKey(wanted.find((e) => e.id.kind === 'mysql')!.id), error: { code: 'network', message: 'connection reset while downloading mysql' } } : undefined,
				// Each package is installed as soon as it is done, like the real downloader.
				onSubjectDone: (subject) => {
					const done = wanted.find((e) => packageKey(e.id) === subject);
					if (done) addPackage(done.id);
					syncPackages();
				},
				onSuccess: () => {
					firstRunCompleted = true;
					settings = { ...settings, firstRunCompleted: true };
					syncPackages();
				}
			});
		},
		cancelTask: async (taskId) => {
			cancelled.add(taskId);
			return true;
		},
		listRunningTasks: async () => [...running.values()].map(clone),
		listPhpExtensions: async (phpVersion) => {
			await sleep(120);
			if (!installed.some((i) => i.kind === 'php' && i.version.startsWith(`${phpVersion}.`))) fail('package-missing', `PHP ${phpVersion} is not installed`);
			// What the official Windows builds ship in `ext\` (older minors lack a few).
			const names = ['bz2', 'curl', 'dba', 'enchant', 'exif', 'ffi', 'fileinfo', 'ftp', 'gd', 'gettext', 'gmp', 'imap', 'intl', 'ldap', 'mbstring', 'mysqli', 'odbc', 'opcache', 'openssl', 'pdo_firebird', 'pdo_mysql', 'pdo_odbc', 'pdo_pgsql', 'pdo_sqlite', 'pgsql', 'shmop', 'snmp', 'soap', 'sockets', 'sodium', 'sqlite3', 'sysvshm', 'tidy', 'xsl', 'zip'];
			const old = Number(phpVersion.split('.')[0]) < 8;
			return names.filter((n) => !(old && (n === 'zip' || n === 'ffi'))).map((name) => ({ name, zend: name === 'opcache' }));
		},

		// Services
		listServices: async () => {
			await sleep(150);
			return [...services.values()].map(clone);
		},
		startServices: async (r = { services: [] }) => {
			const names = targets(r).filter((n) => services.get(n)?.state !== 'running');
			await Promise.all(names.map(startOne));
			return report({ started: names });
		},
		stopServices: async (r = { services: [] }) => {
			const names = targets(r).filter((n) => services.get(n)?.state === 'running');
			await Promise.all(names.map(stopOne));
			return report({ stopped: names });
		},
		restartServices: async (r = { services: [] }) => {
			const names = targets(r);
			await Promise.all(names.map(async (n) => (services.get(n)?.state === 'running' ? (await stopOne(n), startOne(n)) : startOne(n))));
			return report({ restarted: names });
		},
		applyEnvironment: async () => {
			await sleep(1200);
			const updated = !hostsInSync;
			hostsInSync = true;
			return report({ hostsFileUpdated: updated });
		},

		// Worktrees
		listWorktrees: async (projectId = null) => {
			await sleep(200);
			return worktrees.filter((w) => !projectId || w.projectId === projectId).map(clone);
		},
		createWorktree: async (r) => {
			await sleep(100);
			const projectId = r.projectId ?? projects.find((p) => r.path.toLowerCase().includes(p.id))?.id ?? projects[0].id;
			const slug = r.path.split(/[\\/]/).filter(Boolean).pop()!.toLowerCase().replace(/[^a-z0-9]+/g, '-');
			if (worktrees.some((w) => w.site === `${projectId}@${slug}`)) fail('conflict', 'worktree exists');
			const used = new Set(worktrees.map((w) => w.loopbackPort));
			let port = settings.loopbackPortRange.start;
			while (used.has(port)) port++;
			if (port > settings.loopbackPortRange.end) fail('validation', 'no free port in range');
			const mode = r.database ?? { mode: 'shared' as const };
			return startTask({ kind: 'worktree-up', request: r }, [`worktree-${slug}`], {
				onSuccess: () => {
					worktrees = [...worktrees, { site: `${projectId}@${slug}`, projectId, slug, root: r.path, branch: `agent/${slug}`, url: `http://127.0.0.1:${port}`, urls: [`http://127.0.0.1:${port}`], loopbackPort: port, localhostAlias: `${slug}.${projectId}.localhost`, documentRoot: `${r.path}\\public`, databaseMode: mode, database: mode.mode === 'shared' ? null : connection(`${projectId}_${slug.replace(/-/g, '_')}`, '8.4'), stale: false, createdAtUnixMs: Date.now() }];
					syncWorktrees(projectId);
					syncProject(projectId);
				}
			});
		},
		removeWorktree: async ({ target, dropDatabase }) => {
			await sleep(400);
			const w = worktrees.find((x) => x.site === target || x.root === target) ?? fail('not-found', target);
			worktrees = worktrees.filter((x) => x !== w);
			syncWorktrees(w.projectId);
			syncProject(w.projectId);
			return { site: w.site, databaseDropped: dropDatabase && w.database ? w.database.database : null };
		},
		pruneWorktrees: async ({ projectId = null, dryRun = false, dropDatabases = false } = {}) => {
			await sleep(300);
			const stale = worktrees.filter((w) => w.stale && (!projectId || w.projectId === projectId));
			if (!dryRun) {
				worktrees = worktrees.filter((w) => !stale.includes(w));
				for (const id of new Set(stale.map((w) => w.projectId))) {
					syncWorktrees(id);
					syncProject(id);
				}
			}
			return { dryRun, removed: stale.map((w) => w.site), droppedDatabases: dropDatabases ? stale.flatMap((w) => (w.database ? [w.database.database] : [])) : [], failed: [] };
		},

		// Logs
		listLogSources: async (site = null) => {
			await sleep(120);
			const sites = projects.filter((p) => !site || p.id === site);
			const src = (id: string): T.LogSource => ({ id, file: logs.file(id), exists: logs.exists(id) });
			return [
				...(site ? [] : [src('apache'), src('apache-error'), src('mysql-8.4'), src('mysql-8.4-error'), src('mysql-5.7'), src('redis'), src('mailpit')]),
				...sites.flatMap((p) => [src(`access:${p.id}`), src(`error:${p.id}`), src(`php:${p.id}`)])
			];
		},
		tailLog: async ({ source, lines }) => logs.tail(source, lines),
		readLogFrom: async (source, offset) => logs.readFrom(source, offset),
		openLogFile: async (source) => {
			if (!logs.exists(source)) fail('validation', `the log file does not exist yet: ${logs.file(source)}`, { path: logs.file(source) });
		},
		openLogFolder: async () => {},
		exportLogLines: async ({ file }) => {
			await sleep(200);
			if (!file.trim()) fail('validation', 'an export file is required');
		},

		// Settings
		listServiceKinds: async () => {
			await sleep(80);
			const mysql = settings.services.filter((s) => s.service.startsWith('mysql-'));
			return [
				{ kind: 'mysql', label: 'MySQL', versioned: true, versions: mysql.map((s) => s.service.slice(6)), defaultVersion: mysql.find((s) => s.alwaysOn)?.service.slice(6) ?? null },
				{ kind: 'redis', label: 'Redis', versioned: false, versions: [], defaultVersion: null },
				{ kind: 'mailpit', label: 'Mailpit', versioned: false, versions: [], defaultVersion: null }
			];
		},
		getSettings: async () => {
			await sleep(150);
			return clone(settings);
		},
		updateSettings: async (next) => {
			await sleep(300);
			if (next.loopbackPortRange.start >= next.loopbackPortRange.end) fail('validation', 'range');
			settings = clone(next);
			emitter.emit<void>('settings-changed', undefined as never);
			return clone(settings);
		},

		// Diagnostics
		runDoctor: async () => {
			await sleep(900);
			const failed = services.get('mysql-5.7')?.state === 'failed';
			const missingVc: T.VcRuntimeIssue[] = isInstalled({ kind: 'php', version: '5.6.40', phpVersion: null }) ? [{ redistributable: 'Visual C++ 2012 (x64)', downloadUrl: 'https://www.microsoft.com/download/details.aspx?id=30679', requiredBy: ['php-5.6.40'] }] : [];
			const stale = worktrees.filter((w) => w.stale).map((w) => w.site);
			const missingPackages = fresh && !firstRunCompleted ? ['php 8.4', 'mysql 8.4', 'apache 2.4'] : [];
			const down = [...services.values()].filter((s) => needed(s) && (s.state === 'failed' || s.state === 'stopped'));
			const servicesDown = [...down.filter((s) => s.state === 'failed'), ...down.filter((s) => s.state === 'stopped')].map((s) => s.service);
			const servicesStatus: T.CheckStatus = down.some((s) => s.state === 'failed') ? 'error' : down.length ? 'warning' : 'ok';
			// Same ids, order and fix hints as the real `standard_checks`, so every fix action can show up.
			const checks: T.DoctorCheck[] = [
				{ id: 'settings', status: 'ok', message: 'Impostazioni leggibili', fix: null },
				{ id: 'project-files', status: 'ok', message: 'File dei progetti leggibili', fix: null },
				{ id: 'vc-runtime', status: missingVc.length ? 'error' : 'ok', message: missingVc.length ? 'Runtime Visual C++ 2012 mancante (php-5.6.40)' : 'Runtime Visual C++ presenti', fix: missingVc.length ? 'Install the Visual C++ 2012 redistributable' : null },
				{ id: 'ports', status: failed ? 'error' : 'ok', message: failed ? 'La porta 3307 è occupata da mysqld.exe (PID 4412)' : 'Nessun conflitto di porte', fix: failed ? 'Stop mysqld.exe or change the MySQL 5.7 port in settings' : null },
				{ id: 'shims-on-path', status: shims.onPath ? 'ok' : 'warning', message: shims.onPath ? 'Le shim sono nel PATH' : 'Le shim non sono nel PATH', fix: shims.onPath ? null : 'Run install shims' },
				{ id: 'path-shadowing', status: shims.shadowed.length ? 'warning' : 'ok', message: shims.shadowed.length ? 'php è oscurato da C:\\xampp\\php\\php.exe' : 'Nessuno strumento oscurato', fix: null },
				{ id: 'hosts-block', status: hostsInSync ? 'ok' : 'warning', message: hostsInSync ? 'File hosts aggiornato' : 'Il file hosts non è aggiornato', fix: hostsInSync ? null : 'lampo up --allow-elevation' },
				{ id: 'packages', status: missingPackages.length ? 'error' : 'ok', message: missingPackages.length ? `Pacchetti mancanti: ${missingPackages.join(', ')}` : 'Pacchetti presenti', fix: missingPackages.length ? 'lampo package install <kind> <series>' : null },
				{ id: 'services', status: servicesStatus, message: servicesDown.length ? `Servizi non in esecuzione: ${servicesDown.join(', ')}` : 'Servizi in esecuzione', fix: servicesDown.length ? `lampo up ${servicesDown.join(' ')}` : null },
				{ id: 'folders', status: 'ok', message: 'Cartelle dei progetti presenti', fix: null },
				{ id: 'stale-worktrees', status: stale.length ? 'warning' : 'ok', message: stale.length ? `${stale.length} ambiente worktree con cartella mancante` : 'Nessun ambiente orfano', fix: stale.length ? 'lampo worktree prune' : null },
				{ id: 'app', status: 'ok', message: 'App in esecuzione', fix: null }
			];
			return {
				ok: checks.every((c) => c.status !== 'error'),
				checks,
				missingVcRuntimes: missingVc,
				portConflicts: failed ? [{ port: 3307, wantedBy: 'mysql-5.7', owner: 'mysqld.exe (PID 4412)' }] : [],
				shadowedTools: clone(shims.shadowed),
				staleWorktrees: stale,
				missingPackages,
				servicesDown,
				hostsFileInSync: hostsInSync,
				unreadableProjects: []
			};
		},
		hostsInSync: async () => {
			await sleep(80);
			return hostsInSync;
		},
		shimStatus: async () => {
			await sleep(150);
			return clone(shims);
		},
		installShims: async () => {
			await sleep(600);
			shims = { ...shims, installed: true, onPath: true, shadowed: [] };
			return clone(shims);
		},
		uninstallShims: async () => {
			await sleep(400);
			shims = { ...shims, installed: false, onPath: false };
			return clone(shims);
		},
		claudeCodeStatus: async () => {
			await sleep(400);
			return clone(claudeCode);
		},
		installClaudeCode: async () => {
			await sleep(1500);
			claudeCode = { ...claudeCode, state: 'installed', installedVersion: expectedPlugin, updateError: null };
			const steps = [
				step('detect-claude', claudeCode.claudeVersion),
				step('check-cli', claudeCode.cliPath),
				step('write-files', claudeCode.pluginDir),
				step('validate-plugin'),
				step('validate-marketplace'),
				{ step: 'remove-previous-plugin', outcome: 'ignored' as const, detail: 'Plugin "lampo@lampo" not found' },
				step('remove-previous-marketplace'),
				step('add-marketplace'),
				step('install-plugin'),
				step('verify', 'lampo@lampo'),
				step('write-marker', expectedPlugin)
			];
			return { ok: true, steps, status: clone(claudeCode) };
		},
		uninstallClaudeCode: async () => {
			await sleep(900);
			claudeCode = { ...claudeCode, state: 'not-installed', installedVersion: null, updateError: null };
			const steps = [step('detect-claude', claudeCode.claudeVersion), step('remove-plugin'), step('remove-marketplace'), step('remove-files', claudeCode.pluginDir)];
			return { ok: true, steps, status: clone(claudeCode) };
		}
	};

	// The browser has no native pickers: a prompt stands in for them.
	const ask = async (title: string | undefined, fallback: string) => {
		const value = window.prompt(title ?? 'Percorso', fallback);
		return value?.trim() ? value.trim() : null;
	};
	const platform: Platform = {
		pickFolder: ({ title, defaultPath } = {}) => ask(title, defaultPath ?? 'C:\\Dev\\nuovo-sito'),
		pickFile: ({ title, defaultPath } = {}) => ask(title, defaultPath ?? 'C:\\Dev\\shop.lampo.json'),
		pickSaveFile: ({ title, defaultPath } = {}) => ask(title, defaultPath ?? 'C:\\Dev\\export.lampo.json'),
		openExternal: async (url) => void window.open(url, '_blank', 'noopener,noreferrer')
	};

	// A newer release is always available; the download reports progress, the install restarts.
	const MOCK_UPDATE_SIZE = 18_500_000;
	const updater: Updater = {
		currentVersion: async () => '0.1.0-mock',
		check: async () => {
			await sleep(700);
			return {
				version: '0.2.0',
				date: new Date().toISOString(),
				notes: "Lampo v0.2.0 (simulato): aggiornamento automatico dall'app.",
				download: async (onEvent) => {
					onEvent({ kind: 'started', total: MOCK_UPDATE_SIZE });
					for (let done = 0; done < MOCK_UPDATE_SIZE; done += MOCK_UPDATE_SIZE / 20) {
						await sleep(120);
						onEvent({ kind: 'progress', bytes: MOCK_UPDATE_SIZE / 20 });
					}
					onEvent({ kind: 'finished' });
				},
				install: () => sleep(1200)
			};
		},
		relaunch: async () => window.location.reload()
	};

	return { simulated: true, commands, events: emitter.bus(), platform, updater };
}
