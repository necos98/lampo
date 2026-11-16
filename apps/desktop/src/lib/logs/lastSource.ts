/** The part of `Storage` the viewer uses, so tests can pass a map. */
export interface SourceStorage {
	getItem(key: string): string | null;
	setItem(key: string, value: string): void;
}

const LAST_SOURCE_KEY = 'lampo.logs.lastSource';

/** `localStorage`, or `null` where reading it throws (blocked storage, private windows). */
function browserStorage(): SourceStorage | null {
	try {
		return globalThis.localStorage ?? null;
	} catch {
		return null;
	}
}

/** The source the viewer showed last, if it was remembered. */
export function readLastSource(storage: SourceStorage | null = browserStorage()): string | null {
	try {
		return storage?.getItem(LAST_SOURCE_KEY) || null;
	} catch {
		return null;
	}
}

/** Remembers `source` for the next visit; a storage that refuses it only loses the convenience. */
export function rememberLastSource(source: string, storage: SourceStorage | null = browserStorage()): void {
	try {
		storage?.setItem(LAST_SOURCE_KEY, source);
	} catch {
		// Full or blocked storage: the next visit falls back to the first source with a file.
	}
}

/**
 * The source to open first: the one asked for (a link from a project), else the remembered
 * one, else the first that has a file. Asked and remembered sources count only if listed.
 */
export function pickInitialSource(sources: readonly { id: string; exists: boolean }[], wanted: string | null, remembered: string | null): string | null {
	const listed = (id: string | null) => (id ? sources.find((s) => s.id === id)?.id : undefined);
	return listed(wanted) ?? listed(remembered) ?? sources.find((s) => s.exists)?.id ?? null;
}
