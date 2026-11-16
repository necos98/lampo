import type { Project } from '../api/index.js';

/** Fields of a project the text filter of the projects page searches in. */
type FilterableProject = Pick<Project, 'name' | 'id' | 'root' | 'url'>;

/**
 * Projects whose name, id, folder or URL contain `query` (trimmed, case-insensitive).
 * An empty query keeps every project. Other criteria (status, order) compose on its result.
 */
export function filterProjects<T extends FilterableProject>(projects: T[], query: string): T[] {
	const needle = query.trim().toLocaleLowerCase();
	if (!needle) return projects;
	return projects.filter((p) => [p.name, p.id, p.root, p.url].some((field) => field.toLocaleLowerCase().includes(needle)));
}
