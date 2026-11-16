import type { AppError } from '../api/index.js';
import type { EnvVarPart } from './envVarList.js';

/** `details.field` of a validation error about the project's environment variables. */
const ENV_VARS_FIELD = 'envVars';

/** Why the backend refused the environment variables, from the details of its error. */
export interface EnvVarRefusal {
	/** The variable refused, as sent; null when the refusal is about all of them (their count). */
	name: string | null;
	/** Which part of that variable; null with `name`. */
	part: EnvVarPart | null;
	/** The name is one Lampo manages (`PATH`, `PHPRC`, `LAMPO_*`, …). */
	reserved: boolean;
	/** The backend's own (English) reason. Never holds the value. */
	reason: string;
}

/** The refusal carried by `error`, if it is one about the environment variables. */
export function envVarRefusal(error: Pick<AppError, 'code' | 'details'>): EnvVarRefusal | null {
	if (error.code !== 'validation') return null;
	const d = error.details as { field?: unknown; name?: unknown; part?: unknown; reserved?: unknown; reason?: unknown } | undefined;
	if (d?.field !== ENV_VARS_FIELD || typeof d.reason !== 'string') return null;
	const name = typeof d.name === 'string' ? d.name : null;
	const part = name !== null && (d.part === 'name' || d.part === 'value') ? d.part : null;
	return { name, part, reserved: d.reserved === true, reason: d.reason };
}
