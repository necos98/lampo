import type { ProjectEnvironment } from '../api/index.js';

/** The kind whose instance holds the project's database (connection details, database name). */
export const DATABASE_KIND = 'mysql';

/** Version used of `kind` (versioned kinds), null when unused or unversioned. */
export function serviceVersion(env: ProjectEnvironment, kind: string): string | null {
	return env.services.find((s) => s.kind === kind)?.version ?? null;
}

export function usesService(env: ProjectEnvironment, kind: string): boolean {
	return env.services.some((s) => s.kind === kind);
}

/** Uses `kind` (with `version` for versioned kinds), or stops using it when `use` is false. */
export function setService(env: ProjectEnvironment, kind: string, use: boolean, version: string | null = null): void {
	const others = env.services.filter((s) => s.kind !== kind);
	env.services = use ? [...others, { kind, version }].sort((a, b) => (a.kind < b.kind ? -1 : a.kind > b.kind ? 1 : 0)) : others;
}
