/**
 * Common extensions of the Windows PHP builds: the fallback offered as checkboxes when the
 * project's PHP minor is not installed (otherwise `listPhpExtensions` says what it ships).
 * Extensions already enabled on a project are always shown (see `extensionChoices`).
 */
export const KNOWN_EXTENSIONS: readonly string[] = [
	'bcmath',
	'curl',
	'exif',
	'fileinfo',
	'gd',
	'gettext',
	'gmp',
	'intl',
	'ldap',
	'mbstring',
	'mysqli',
	'opcache',
	'openssl',
	'pdo_mysql',
	'pdo_sqlite',
	'soap',
	'sockets',
	'sodium',
	'sqlite3',
	'xml',
	'zip'
];

/** `available` (or the common list when unknown) plus any enabled ones it lacks, sorted. */
export function extensionChoices(enabled: readonly string[], available: readonly string[] | null = null): string[] {
	return [...new Set([...(available ?? KNOWN_EXTENSIONS), ...enabled])].sort();
}

