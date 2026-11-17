import type { CatalogEntry, InstalledPackage, PackageId, UpdateAvailable } from '../api/index.js';

/** One installable line: a series of a package kind (PHP 8.3, MySQL 8.4, Xdebug for PHP 8.3). */
export interface PackageRow {
	key: string;
	kind: string;
	/** Tab the backend assigns (`php`, `mysql`, `addons`, `tools`, …). */
	group: string;
	series: string;
	phpVersion: string | null;
	/** Newest catalog entry of the series. */
	latest: CatalogEntry;
	/** Installed patch of the series, if any. */
	installed: InstalledPackage | null;
	update: UpdateAvailable | null;
	endOfLife: boolean;
}

export const rowKey = (kind: string, series: string, phpVersion: string | null) => `${kind}|${series}|${phpVersion ?? ''}`;
export const installTaskKey = (kind: string, series: string, phpVersion: string | null) => `install:${rowKey(kind, series, phpVersion)}`;
/** Key of the uninstall task of exactly one installed build. */
export const uninstallTaskKey = (id: PackageId) => `uninstall:${id.kind}|${id.version}|${id.phpVersion ?? ''}`;
/** Whether `taskKey` installs or uninstalls a package of `kind` (not an add-on built for it). */
export const isPackageTaskOf = (taskKey: string, kind: string) => [`install:${kind}|`, `uninstall:${kind}|`].some((prefix) => taskKey.startsWith(prefix));

/**
 * One row per kind + series (+ PHP minor). Series, groups and ordering all come from the backend:
 * the catalog arrives newest first within each kind and series, so the first entry of a row is
 * its latest and rows keep the backend's order.
 */
export function buildRows(catalog: CatalogEntry[], installed: InstalledPackage[], updates: UpdateAvailable[]): PackageRow[] {
	const rows = new Map<string, PackageRow>();
	for (const e of catalog) {
		const key = rowKey(e.id.kind, e.series, e.id.phpVersion);
		if (rows.has(key)) continue;
		rows.set(key, {
			key,
			kind: e.id.kind,
			group: e.group,
			series: e.series,
			phpVersion: e.id.phpVersion,
			latest: e,
			installed: installed.find((i) => rowKey(i.id.kind, i.series, i.id.phpVersion) === key) ?? null,
			update: updates.find((u) => rowKey(u.kind, u.series, u.phpVersion) === key) ?? null,
			endOfLife: e.endOfLife
		});
	}
	return [...rows.values()];
}

/**
 * The groups present in `rows`, in `preferred` order first, then any group the backend added later
 * (the "other" fallback: shown under its own id until the GUI gets a label for it).
 */
export function groupsOf(rows: PackageRow[], preferred: readonly string[]): string[] {
	const present = [...new Set(rows.map((r) => r.group))];
	return [...preferred.filter((g) => present.includes(g)), ...present.filter((g) => !preferred.includes(g))];
}
