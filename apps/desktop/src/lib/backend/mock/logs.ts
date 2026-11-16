import type * as T from '../../api/types.js';

const BASE_LINES = 240;
const TICK_MS = 900;
/** Sources whose file the mock pretends was never written. */
const MISSING_SOURCES: ReadonlySet<string> = new Set(['mysql-5.7', 'php:blog']);

const TEMPLATES: Record<string, string[]> = {
	apache: ['[core:notice] AH00094: Command line: httpd.exe -d C:/Lampo/apache', '[mpm_winnt:notice] AH00418: Parent: Created child process 7712', '[mpm_winnt:notice] AH00354: Child: Starting 64 worker threads.'],
	error: [
		'[php:notice] Notice: Undefined index: page in {root}\\index.php on line 41',
		'[php:warn] Warning: Cannot modify header information - headers already sent in {root}\\bootstrap.php on line 9',
		'[core:error] AH00126: Invalid URI in request GET /..%2f HTTP/1.1',
		'[php:error] PHP Fatal error: Uncaught Error: Call to undefined function mb_strlen() in {root}\\app\\Str.php:18',
		'[proxy_fcgi:info] mod_fcgid: process 5512 exited normally'
	],
	access: [
		'127.0.0.1 - - "GET / HTTP/1.1" 200 5123',
		'127.0.0.1 - - "GET /assets/app.css HTTP/1.1" 200 20844',
		'127.0.0.1 - - "POST /api/cart HTTP/1.1" 201 312',
		'127.0.0.1 - - "GET /favicon.ico HTTP/1.1" 404 196',
		'127.0.0.1 - - "GET /checkout HTTP/1.1" 500 1021'
	],
	php: ['PHP Deprecated: Return type of Foo::count() should be compatible in {root}\\src\\Foo.php on line 22', 'PHP Warning: Undefined variable $user in {root}\\views\\nav.php on line 7', 'PHP Notice: A non well formed numeric value encountered in {root}\\lib\\money.php on line 130'],
	mysql: ['[System] [MY-010931] [Server] ready for connections. port: 3306', '[Warning] [MY-010055] [Server] IP address could not be resolved', '[Note] [MY-013386] [Server] Running queries to upgrade MySQL server.'],
	redis: ['* Ready to accept connections tcp', '- DB 0: 3 keys (0 volatile) in 4 slots HT.', '* 1 changes in 3600 seconds. Saving...'],
	mailpit: ['[smtpd] starting on 127.0.0.1:1025 (no encryption)', '[http] starting on 127.0.0.1:8025', '[smtpd] message received from shop@localhost via 127.0.0.1 (1 recipient)']
};

function family(source: string): keyof typeof TEMPLATES {
	if (source.startsWith('access:')) return 'access';
	if (source.startsWith('php:')) return 'php';
	if (source.startsWith('error:') || source.endsWith('-error') || source === 'apache-error') return 'error';
	if (source.startsWith('mysql')) return 'mysql';
	if (source.startsWith('redis')) return 'redis';
	if (source.startsWith('mailpit')) return 'mailpit';
	return 'apache';
}

/** Deterministic pseudo-log: line `i` of `source` never changes. */
function lineAt(source: string, i: number, t0: number): T.LogLine {
	const templates = TEMPLATES[family(source)];
	const site = source.includes(':') ? source.split(':')[1] : 'site';
	const at = t0 + (i - BASE_LINES) * TICK_MS;
	const stamp = new Date(at).toISOString().slice(11, 19);
	const text = templates[(i * 7 + source.length) % templates.length].replace('{root}', `C:\\Dev\\${site}`);
	return { text: `[${stamp}] ${text}`, atUnixMs: at };
}

export class MockLogs {
	readonly #t0 = Date.now();

	#available(): number {
		return BASE_LINES + Math.floor((Date.now() - this.#t0) / TICK_MS);
	}

	file(source: string): string {
		return `C:\\Users\\dev\\AppData\\Local\\Lampo\\logs\\${source.replace(':', '-')}.log`;
	}

	exists(source: string): boolean {
		return !MISSING_SOURCES.has(source);
	}

	tail(source: string, count: number): T.LogLines {
		const end = this.#available();
		const from = Math.max(0, end - count);
		const lines: T.LogLine[] = [];
		for (let i = from; i < end; i++) lines.push(lineAt(source, i, this.#t0));
		return { source, file: this.file(source), lines, nextOffset: end };
	}

	readFrom(source: string, offset: number): T.LogLines {
		const end = this.#available();
		const lines: T.LogLine[] = [];
		for (let i = offset; i < end; i++) lines.push(lineAt(source, i, this.#t0));
		return { source, file: this.file(source), lines, nextOffset: end };
	}
}
