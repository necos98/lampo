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

/** Raw tool output attached to a `config-rejected` error, if any. */
export function errorOutput(value: unknown): string | null {
	const err: AppError = toAppError(value);
	const d = err.details as { output?: unknown } | undefined;
	return typeof d?.output === 'string' ? d.output : null;
}
