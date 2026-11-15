import type * as T from '../../api/types.js';

/** Test shorthand: `mysql` / `redis` become entries of `services`, as the backend stores them. */
type EnvOverrides = Partial<T.ProjectEnvironment> & { mysql?: string | null; redis?: boolean };

const env = ({ mysql = '8.4', redis = false, ...over }: EnvOverrides): T.ProjectEnvironment => ({
	phpVersion: '8.4',
	extensions: ['curl', 'mbstring', 'openssl', 'pdo_mysql', 'mysqli', 'fileinfo'],
	addons: [],
	phpIni: {},
	publicDir: 'public',
	services: [...(mysql ? [{ kind: 'mysql', version: mysql }] : []), ...(redis ? [{ kind: 'redis', version: null }] : [])],
	apacheDirectives: '',
	...over
});

/** The MySQL series a (mock) environment uses. */
export const mysqlOf = (e: T.ProjectEnvironment): string | null => e.services.find((s) => s.kind === 'mysql')?.version ?? null;

/** What the backend computes for catalog entries: the series and the GUI group. */
export const seriesOf = (version: string): string => version.split('.').slice(0, 2).join('.');
const SERVICE_PACKAGES = ['apache', 'mysql', 'redis'];
export const groupOf = (id: T.PackageId): string => (id.phpVersion ? 'addons' : id.kind === 'php' ? 'php' : SERVICE_PACKAGES.includes(id.kind) ? id.kind : 'tools');

export function seedPresets(): T.Preset[] {
	return [
		{ id: 'laravel', name: 'Laravel', description: '', builtin: true, environment: env({ extensions: ['curl', 'mbstring', 'openssl', 'pdo_mysql', 'fileinfo', 'bcmath', 'zip'] }) },
		{ id: 'wordpress', name: 'WordPress', description: '', builtin: true, environment: env({ publicDir: '', phpVersion: '8.3', extensions: ['curl', 'mbstring', 'mysqli', 'gd', 'zip', 'exif'] }) },
		{ id: 'symfony', name: 'Symfony', description: '', builtin: true, environment: env({ extensions: ['curl', 'mbstring', 'intl', 'pdo_mysql', 'xml'] }) },
		{ id: 'generic-php', name: 'Generic PHP', description: '', builtin: true, environment: env({ publicDir: '', mysql: null }) },
		{ id: 'legacy', name: 'Legacy', description: '', builtin: true, environment: env({ phpVersion: '7.4', publicDir: '', mysql: '5.7', extensions: ['mysqli', 'mbstring', 'curl'] }) },
		{ id: 'agency-base', name: 'Agency base', description: 'Il mio setup per i clienti', builtin: false, environment: env({ phpVersion: '8.3', redis: true, addons: ['xdebug'] }) }
	];
}

export function connection(database: string, mysql: string): T.DatabaseConnection {
	return { mysqlVersion: mysql, host: '127.0.0.1', port: mysql === '8.4' ? 3306 : 3307, user: 'root', password: '', database };
}

function project(id: string, name: string, root: string, host: string, e: T.ProjectEnvironment, over: Partial<T.Project> = {}): T.Project {
	const db = id.replace(/-/g, '_');
	return {
		id,
		name,
		root,
		folderExists: true,
		addresses: [
			{ kind: 'hostname', host },
			{ kind: 'localhost', host: `${id}.localhost` }
		],
		url: `http://${host}`,
		urls: [`http://${host}`, `http://${id}.localhost`],
		documentRoot: e.publicDir ? `${root}\\${e.publicDir}` : root,
		environment: e,
		databaseName: mysqlOf(e) ? db : null,
		database: mysqlOf(e) ? connection(db, mysqlOf(e) as string) : null,
		tls: 'disabled',
		active: true,
		originPreset: null,
		worktrees: [],
		...over
	};
}

export function seedProjects(): T.Project[] {
	return [
		project('shop', 'Shop', 'C:\\Dev\\shop', 'shop.test', env({ addons: ['xdebug'], redis: true }), { originPreset: 'laravel' }),
		project('blog', 'Blog aziendale', 'C:\\Dev\\blog', 'blog.test', env({ phpVersion: '8.3', publicDir: '', extensions: ['curl', 'mbstring', 'mysqli', 'gd', 'zip'] }), { originPreset: 'wordpress' }),
		project('crm-legacy', 'CRM legacy', 'C:\\Dev\\crm-legacy', 'crm.test', env({ phpVersion: '5.6', publicDir: '', mysql: '5.7', extensions: ['mysqli', 'mbstring'] }), { originPreset: 'legacy' }),
		project('api-symfony', 'API Symfony', 'D:\\Work\\api-symfony', 'api.test', env({ phpVersion: '8.4', extensions: ['intl', 'pdo_mysql', 'xml'] }), { active: false, originPreset: 'symfony' }),
		project('vecchio-sito', 'Vecchio sito', 'C:\\Dev\\vecchio-sito', 'vecchio.test', env({ phpVersion: '8.2', publicDir: '', mysql: null }), { folderExists: false, active: false })
	];
}

export function seedWorktrees(): T.WorktreeEnvironment[] {
	const wt = (projectId: string, slug: string, branch: string | null, port: number, stale = false, mode: T.DatabaseMode = { mode: 'fresh' }): T.WorktreeEnvironment => ({
		site: `${projectId}@${slug}`,
		projectId,
		slug,
		root: `C:\\Dev\\.worktrees\\${projectId}\\${slug}`,
		branch,
		url: `http://127.0.0.1:${port}`,
		urls: [`http://127.0.0.1:${port}`, `http://${slug}.${projectId}.localhost:${port}`],
		loopbackPort: port,
		localhostAlias: `${slug}.${projectId}.localhost`,
		documentRoot: `C:\\Dev\\.worktrees\\${projectId}\\${slug}\\public`,
		databaseMode: mode,
		database: mode.mode === 'shared' ? null : connection(`${projectId}_${slug.replace(/-/g, '_')}`, '8.4'),
		stale,
		createdAtUnixMs: Date.now() - 86_400_000 * (port - 8100)
	});
	return [
		wt('shop', 'feature-login', 'feature/login', 8101),
		wt('shop', 'fix-cart', 'fix/cart-total', 8102, false, { mode: 'clone', source: 'shop' }),
		wt('shop', 'agent-7f3a', null, 8103, true, { mode: 'shared' })
	];
}

const pid = (kind: string, version: string, phpVersion: string | null = null): T.PackageId => ({ kind, version, phpVersion });

/** `kind-version[-php]`, the identity of one package version. */
export const packageKey = (p: T.PackageId): string => `${p.kind}-${p.version}${p.phpVersion ? `-${p.phpVersion}` : ''}`;

export function seedCatalog(): { catalog: T.CatalogEntry[]; installed: T.PackageId[] } {
	const entry = (id: T.PackageId, over: Partial<T.CatalogEntry> = {}): T.CatalogEntry => ({
		id,
		channel: 'live',
		endOfLife: false,
		sizeBytes: 28_000_000,
		vcRuntime: null,
		vcRuntimeInstalled: null,
		installed: false,
		series: seriesOf(id.version),
		group: groupOf(id),
		...over
	});
	const phps: [string, string, string, boolean][] = [
		['5.6', '5.6.40', 'vc11', true],
		['7.0', '7.0.33', 'vc14', true],
		['7.1', '7.1.33', 'vc14', true],
		['7.2', '7.2.34', 'vc15', true],
		['7.3', '7.3.33', 'vc15', true],
		['7.4', '7.4.33', 'vc15', true],
		['8.0', '8.0.30', 'vs16', true],
		['8.1', '8.1.31', 'vs16', true],
		['8.2', '8.2.26', 'vs16', false],
		['8.3', '8.3.14', 'vs16', false],
		['8.4', '8.4.1', 'vs17', false],
		['8.5', '8.5.0', 'vs17', false]
	];
	const catalog: T.CatalogEntry[] = [];
	for (const [series, version, vc, eol] of phps) {
		catalog.push(
			entry(pid('php', version), {
				endOfLife: eol,
				channel: eol ? 'frozen' : 'live',
				sizeBytes: 31_000_000,
				vcRuntime: vc,
				vcRuntimeInstalled: vc !== 'vc11'
			})
		);
		if (parseFloat(series) >= 7.4) catalog.push(entry(pid('xdebug', '3.4.0', series), { sizeBytes: 3_000_000 }));
		if (parseFloat(series) >= 7.0) catalog.push(entry(pid('php-redis', '6.1.0', series), { sizeBytes: 600_000 }));
	}
	catalog.push(
		entry(pid('php', '8.3.12'), { sizeBytes: 31_000_000, vcRuntime: 'vs16', vcRuntimeInstalled: true }),
		entry(pid('apache', '2.4.62'), { sizeBytes: 11_000_000, vcRuntime: 'vs17', vcRuntimeInstalled: true }),
		entry(pid('mod-fcgid', '2.3.9'), { sizeBytes: 200_000 }),
		entry(pid('mysql', '8.4.3'), { sizeBytes: 220_000_000, vcRuntime: 'vs17', vcRuntimeInstalled: true }),
		entry(pid('mysql', '5.7.44'), { endOfLife: true, channel: 'frozen', sizeBytes: 380_000_000, vcRuntime: 'vc15', vcRuntimeInstalled: true }),
		entry(pid('redis', '7.4.1'), { sizeBytes: 9_000_000 }),
		entry(pid('composer', '2.8.3'), { sizeBytes: 2_900_000 })
	);
	const installed = [
		pid('php', '8.4.1'),
		pid('php', '8.3.12'),
		pid('php', '7.4.33'),
		pid('php', '5.6.40'),
		pid('apache', '2.4.62'),
		pid('mod-fcgid', '2.3.9'),
		pid('mysql', '8.4.3'),
		pid('composer', '2.8.3'),
		pid('xdebug', '3.4.0', '8.4')
	];
	return { catalog, installed };
}
