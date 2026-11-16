import { toAppError, type AppError } from '../api/index.js';
import { messages as m } from '../messages/index.js';

interface PortDetails {
	port?: number;
	owner?: string | null;
}

/** Italian, user-facing text for an error; details enrich the generic message when known. */
export function describeError(value: unknown): string {
	const err = toAppError(value);
	if (err.code === 'port-in-use') {
		const d = (err.details ?? {}) as PortDetails;
		if (d.port !== undefined) return m.errorDetails.portInUse(d.port, d.owner ?? null);
	}
	return m.errors[err.code];
}

/** Raw program output attached to an error (`details.output`: `httpd -t` for `config-rejected`,
 *  the last lines of a failed clone or Laravel Installer for `process-failed`), if any. */
export function errorOutput(value: unknown): string | null {
	const err: AppError = toAppError(value);
	const d = err.details as { output?: unknown } | undefined;
	return typeof d?.output === 'string' && d.output.trim() !== '' ? d.output : null;
}

/** `details.field` of a validation error about the project's custom Apache directives. */
const APACHE_DIRECTIVES_FIELD = 'apacheDirectives';

/** Why Lampo refused the custom Apache directives (which directive, which line), if it did. */
export function refusedDirectivesReason(value: unknown): string | null {
	const err: AppError = toAppError(value);
	if (err.code !== 'validation') return null;
	const d = err.details as { field?: unknown; reason?: unknown } | undefined;
	return d?.field === APACHE_DIRECTIVES_FIELD && typeof d.reason === 'string' ? d.reason : null;
}
