import type { ErrorCode, Project } from '../api/index.js';

/** The backend saved the change but the hosts file still needs the administrator prompt. */
const ELEVATION_REQUIRED: ErrorCode = 'elevation-required';

/** What «Attiva tutti» / «Disattiva tutti» did, project by project. */
export interface BulkActivation {
	/** Ids whose state was saved (including the ones waiting for the hosts file). */
	changed: string[];
	/** Ids the backend refused, with its rejection. */
	failed: { id: string; error: unknown }[];
	/** At least one change waits for «Applica modifiche» (hosts file). */
	elevationPending: boolean;
}

/**
 * Ids of the projects a bulk switch to `active` would change: the ones not already in that state.
 * A project whose folder is missing cannot be served, so it is left out (its switch is disabled).
 */
export function bulkActivationTargets(projects: Pick<Project, 'id' | 'active' | 'folderExists'>[], active: boolean): string[] {
	return projects.filter((p) => p.folderExists && p.active !== active).map((p) => p.id);
}

const isElevationRequired = (e: unknown) => typeof e === 'object' && e !== null && (e as { code?: unknown }).code === ELEVATION_REQUIRED;

/**
 * Applies `setOne` to each id, one at a time (each change rewrites Apache's configuration and maybe
 * the hosts file: running them together would only race). A refusal does not stop the others: it is
 * collected so the caller can say which projects were left as they were.
 */
export async function setEachActive(ids: readonly string[], setOne: (id: string) => Promise<unknown>): Promise<BulkActivation> {
	const result: BulkActivation = { changed: [], failed: [], elevationPending: false };
	for (const id of ids) {
		try {
			await setOne(id);
			result.changed.push(id);
		} catch (error) {
			if (isElevationRequired(error)) {
				result.changed.push(id);
				result.elevationPending = true;
			} else result.failed.push({ id, error });
		}
	}
	return result;
}
