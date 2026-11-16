/**
 * The backend's checks of a project's environment variables (`lampo_domain::env_var`), enough
 * to show its refusals in the mock. Like the backend, a refusal names the variable, never its value.
 */

const MAX_COUNT = 256;
const NAME_SYNTAX = /^[A-Za-z_][A-Za-z0-9_]{0,127}$/;
const RESERVED_NAMES = ['phprc', 'php_ini_scan_dir', 'path', 'php_fcgi_max_requests', 'php_fcgi_children', 'temp', 'tmp'];
const RESERVED_PREFIX = 'lampo_';

/** `details` of the backend's `validation` error, as `AppError::from(DomainError)` builds them. */
export interface EnvVarsRefusal {
	message: string;
	details: { field: 'envVars'; reason: string; name?: string; part?: 'name' | 'value'; reserved?: boolean };
}

function isReserved(name: string): boolean {
	const lower = name.toLowerCase();
	return RESERVED_NAMES.includes(lower) || lower.startsWith(RESERVED_PREFIX);
}

function nameRefusal(name: string, reason: string): EnvVarsRefusal {
	return { message: `invalid environment variable name \`${name}\`: ${reason}`, details: { field: 'envVars', name, part: 'name', reserved: isReserved(name), reason } };
}

function valueRefusal(name: string, reason: string): EnvVarsRefusal {
	return { message: `invalid value of the environment variable \`${name}\`: ${reason}`, details: { field: 'envVars', name, part: 'value', reason } };
}

function valueProblem(value: string): string | null {
	if (/[\r\n]/.test(value)) return 'it must be a single line';
	if (value.includes('${')) return 'it contains `${`, which Apache would expand';
	return null;
}

/** The first reason the backend would refuse `vars` for, or null when it would take them. */
export function checkEnvVars(vars: Readonly<Record<string, string>>): EnvVarsRefusal | null {
	const entries = Object.entries(vars);
	if (entries.length > MAX_COUNT) {
		const reason = `at most ${MAX_COUNT} variables per project`;
		return { message: `invalid environment variables: ${reason}`, details: { field: 'envVars', reason } };
	}
	const seen = new Set<string>();
	for (const [rawName, value] of entries) {
		const name = rawName.trim();
		if (!NAME_SYNTAX.test(name)) return nameRefusal(name, 'use letters, digits and `_`, not starting with a digit, at most 128 characters');
		if (isReserved(name)) return nameRefusal(name, 'it is managed by Lampo');
		if (seen.has(name.toLowerCase())) return nameRefusal(name, 'it is given twice (names are case-insensitive)');
		seen.add(name.toLowerCase());
		const problem = valueProblem(value);
		if (problem) return valueRefusal(name, problem);
	}
	return null;
}
