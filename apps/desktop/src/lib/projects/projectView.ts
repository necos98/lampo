import type { Project } from '../api/index.js';

/** Where a project stands, as the list shows it: a missing folder wins over the active flag. */
export type ProjectStatus = 'active' | 'inactive' | 'missing';

/** `all` keeps every project; otherwise only the projects in that status. */
export type ProjectStatusFilter = 'all' | ProjectStatus;

/** `name`: alphabetical. `status`: active, then inactive, then missing folder; by name within each. */
export type ProjectSort = 'name' | 'status';

/** How the projects page orders and narrows its list, remembered between sessions. */
export interface ProjectView {
	sort: ProjectSort;
	status: ProjectStatusFilter;
}

export const PROJECT_SORTS: readonly ProjectSort[] = ['name', 'status'];
export const PROJECT_STATUS_FILTERS: readonly ProjectStatusFilter[] = ['all', 'active', 'inactive', 'missing'];
export const DEFAULT_PROJECT_VIEW: Readonly<ProjectView> = { sort: 'name', status: 'all' };

/** `localStorage` key of the remembered view. */
export const PROJECT_VIEW_KEY = 'lampo.projects.view';

const STATUS_RANK: Record<ProjectStatus, number> = { active: 0, inactive: 1, missing: 2 };

type ViewableProject = Pick<Project, 'name' | 'id' | 'active' | 'folderExists'>;

export function projectStatus(project: Pick<Project, 'active' | 'folderExists'>): ProjectStatus {
	if (!project.folderExists) return 'missing';
	return project.active ? 'active' : 'inactive';
}

export function filterByStatus<T extends ViewableProject>(projects: T[], status: ProjectStatusFilter): T[] {
	return status === 'all' ? projects : projects.filter((p) => projectStatus(p) === status);
}

const collator = new Intl.Collator('it', { sensitivity: 'base', numeric: true });
const byName = (a: ViewableProject, b: ViewableProject) => collator.compare(a.name, b.name) || collator.compare(a.id, b.id);

/** A sorted copy: the input (the store's list) is never reordered in place. */
export function sortProjects<T extends ViewableProject>(projects: T[], sort: ProjectSort): T[] {
	const compare = sort === 'status' ? (a: T, b: T) => STATUS_RANK[projectStatus(a)] - STATUS_RANK[projectStatus(b)] || byName(a, b) : byName;
	return [...projects].sort(compare);
}

const isOneOf = <V extends string>(values: readonly V[], value: unknown): value is V => typeof value === 'string' && (values as readonly string[]).includes(value);

/** The stored view, field by field: anything missing, unknown or malformed falls back to the default. */
export function parseProjectView(raw: string | null): ProjectView {
	let stored: unknown = null;
	try {
		stored = raw === null ? null : JSON.parse(raw);
	} catch {
		// Edited or corrupted by hand: the defaults apply.
	}
	const fields = typeof stored === 'object' && stored !== null ? (stored as Record<string, unknown>) : {};
	return {
		sort: isOneOf(PROJECT_SORTS, fields.sort) ? fields.sort : DEFAULT_PROJECT_VIEW.sort,
		status: isOneOf(PROJECT_STATUS_FILTERS, fields.status) ? fields.status : DEFAULT_PROJECT_VIEW.status
	};
}

type ViewStorage = Pick<Storage, 'getItem' | 'setItem'>;

/** `localStorage`, or `null` where it does not exist; even reaching it can throw (blocked site data). */
const browserStorage = (): ViewStorage | null => (typeof localStorage === 'undefined' ? null : localStorage);

/**
 * The remembered view, or the default when storage is unavailable or refuses the read. The view
 * is a convenience: losing it must never break the page.
 */
export function loadProjectView(storage: () => ViewStorage | null = browserStorage): ProjectView {
	try {
		return parseProjectView(storage()?.getItem(PROJECT_VIEW_KEY) ?? null);
	} catch {
		return { ...DEFAULT_PROJECT_VIEW };
	}
}

/** Remembers the view; `false` when storage is unavailable or refuses the write (quota, private mode). */
export function saveProjectView(view: ProjectView, storage: () => ViewStorage | null = browserStorage): boolean {
	try {
		const target = storage();
		if (!target) return false;
		target.setItem(PROJECT_VIEW_KEY, JSON.stringify({ sort: view.sort, status: view.status }));
		return true;
	} catch {
		return false;
	}
}
