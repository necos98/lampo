import type { AppError, ErrorCode } from './types.js';

const CODES: readonly ErrorCode[] = [
	'validation',
	'not-found',
	'conflict',
	'project-not-resolved',
	'elevation-required',
	'elevation-denied',
	'app-unavailable',
	'package-missing',
	'network',
	'checksum-mismatch',
	'config-rejected',
	'port-in-use',
	'process-failed',
	'io',
	'cancelled',
	'timeout',
	'unsupported',
	'protocol-mismatch',
	'internal'
];

export function isAppError(value: unknown): value is AppError {
	return (
		typeof value === 'object' &&
		value !== null &&
		'code' in value &&
		'message' in value &&
		CODES.includes((value as { code: ErrorCode }).code)
	);
}

/** Normalises anything a command promise may reject with into an `AppError`. */
export function toAppError(value: unknown): AppError {
	if (isAppError(value)) return value;
	return { code: 'internal', message: value instanceof Error ? value.message : String(value) };
}
