import type { ErrorCode } from '../api/index.js';
import type { Messages } from '../messages/index.js';

/** An error as the backend reports it. `code` is a string: a newer backend may send one this GUI
 *  does not know yet, whose English `message` is then the best text there is. */
export interface ReportedError {
	code: string;
	message: string;
	details?: unknown;
}

export type ErrorMessages = Pick<Messages, 'errors' | 'errorDetails'>;

type Details = Readonly<Record<string, unknown>>;

/** `true` for anything shaped like a backend error, known code or not. */
export function isReportedError(value: unknown): value is ReportedError {
	if (typeof value !== 'object' || value === null) return false;
	const { code, message } = value as Record<string, unknown>;
	return typeof code === 'string' && typeof message === 'string';
}

/**
 * User-facing text for a backend error, keyed by `code` and enriched with its `details` (port,
 * site, package…) when they are there; the English `message` for a code with no text.
 */
export function localizeError(error: ReportedError, m: ErrorMessages): string {
	if (!Object.hasOwn(m.errors, error.code)) return error.message;
	const code = error.code as ErrorCode;
	return detailedText(code, asDetails(error.details), m.errorDetails) ?? m.errors[code];
}

/** The text built from `details`, or `null` when they lack what the code's text needs. */
function detailedText(code: ErrorCode, d: Details, t: ErrorMessages['errorDetails']): string | null {
	switch (code) {
		case 'port-in-use': {
			const port = d.port;
			return typeof port === 'number' ? t.portInUse(port, text(d.owner)) : null;
		}
		case 'not-found': {
			const entity = text(d.entity);
			const id = text(d.id);
			const label = entity !== null && Object.hasOwn(t.entities, entity) ? t.entities[entity as keyof typeof t.entities] : null;
			return label !== null && id !== null ? t.notFound(label, id) : null;
		}
		case 'package-missing':
			return packageMissingText(d, t);
		case 'conflict': {
			const pkg = text(d.package);
			const usedBy = textList(d.usedBy);
			return pkg !== null && usedBy !== null ? t.packageInUse(pkg, usedBy) : null;
		}
		case 'config-rejected': {
			const sites = textList(d.excludedSites);
			return sites !== null ? t.sitesNotServed(sites) : null;
		}
		case 'validation': {
			const folder = text(d.folder);
			return folder !== null ? t.folderMissing(folder) : null;
		}
		default:
			return null;
	}
}

function packageMissingText(d: Details, t: ErrorMessages['errorDetails']): string | null {
	const tool = text(d.tool);
	if (tool !== null && Object.hasOwn(t.toolMissing, tool)) return t.toolMissing[tool as keyof typeof t.toolMissing];
	const pkg = text(d.package);
	if (pkg === null) return null;
	const redistributable = text(d.redistributable);
	if (redistributable !== null) return t.runtimeMissing(pkg, redistributable);
	const php = text(d.php);
	return php !== null ? t.addonMissing(pkg, php) : t.packageMissing(pkg);
}

function asDetails(details: unknown): Details {
	return typeof details === 'object' && details !== null && !Array.isArray(details) ? (details as Details) : {};
}

/** A non-blank string value, else `null`. */
function text(value: unknown): string | null {
	return typeof value === 'string' && value.trim() !== '' ? value : null;
}

/** A non-empty list of strings, joined for a sentence; else `null`. */
function textList(value: unknown): string | null {
	if (!Array.isArray(value)) return null;
	const items = value.filter((v): v is string => text(v) !== null);
	return items.length > 0 ? items.join(', ') : null;
}
