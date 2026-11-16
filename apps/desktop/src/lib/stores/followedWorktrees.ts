/**
 * The worktree folders whose creation the user started from a project's panel, so the panel can
 * show their progress or failure. It lives in the store, not in the panel, so it survives the panel
 * being unmounted (changing tab or page). `byProject` stores the folders; pass a `$state` object so
 * `forProject` is reactive in the UI.
 */
export class FollowedWorktrees {
	readonly #byProject: Record<string, string[]>;

	constructor(byProject: Record<string, string[]> = {}) {
		this.#byProject = byProject;
	}

	forProject(projectId: string): readonly string[] {
		return this.#byProject[projectId] ?? [];
	}

	/** Starting the same folder again keeps a single entry. */
	follow(projectId: string, path: string): void {
		const paths = this.forProject(projectId);
		if (!paths.includes(path)) this.#byProject[projectId] = [...paths, path];
	}

	unfollow(projectId: string, path: string): void {
		const rest = this.forProject(projectId).filter((p) => p !== path);
		if (rest.length > 0) this.#byProject[projectId] = rest;
		else delete this.#byProject[projectId];
	}
}
