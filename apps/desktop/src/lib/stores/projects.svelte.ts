import type {
	AddressSuggestions,
	CreateProjectRequest,
	ExportProjectRequest,
	ImportProjectRequest,
	Project,
	ProjectList,
	RelinkProjectRequest,
	UnreadableProject,
	UpdateProjectRequest
} from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { bulkActivationTargets, setEachActive, type BulkActivation } from '../projects/bulkActivation.js';
import { projectWarningsText } from '../projects/warnings.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner, Outcome } from './runner.js';

const EVENT_COALESCE_MS = 150;

export class ProjectsStore {
	readonly list: Resource<ProjectList>;
	/** Projects whose active toggle is being applied. */
	toggling = $state<Record<string, boolean>>({});
	/** «Attiva tutti» / «Disattiva tutti» is being applied. */
	bulkToggling = $state(false);

	readonly #backend: Backend;
	readonly #runner: CommandRunner;
	#timer: ReturnType<typeof setTimeout> | undefined;

	constructor(backend: Backend, runner: CommandRunner) {
		this.#backend = backend;
		this.#runner = runner;
		this.list = new Resource(() => backend.commands.listProjects());
	}

	load = () => this.list.load();

	/** The projects that load (empty until the first load). */
	get all(): Project[] {
		return this.list.data?.projects ?? [];
	}

	/** Project files that do not load: their projects are hidden until the file is repaired. */
	get unreadable(): UnreadableProject[] {
		return this.list.data?.unreadable ?? [];
	}

	/** Default addresses and a free loopback port for `id`, from the backend (`null` if it failed). */
	async suggestAddresses(id: string): Promise<AddressSuggestions | null> {
		try {
			return await this.#backend.commands.suggestAddresses(id);
		} catch {
			return null;
		}
	}

	get(id: string): Project | null {
		return this.all.find((p) => p.id === id) ?? null;
	}

	async setActive(id: string, active: boolean): Promise<Outcome<Project>> {
		this.toggling[id] = true;
		this.#patchActive(id, active);
		const result = await this.#runner.run(() => this.#backend.commands.setProjectActive({ projectId: id, active }), { retry: () => this.setActive(id, active) });
		delete this.toggling[id];
		// On failure (or a pending elevation) the backend state is the truth.
		if (!result.ok) await this.list.load();
		return result;
	}

	/** How many projects «Attiva tutti» (`true`) or «Disattiva tutti» would change. */
	bulkTargetCount(active: boolean): number {
		return this.#bulkTargets(active).length;
	}

	/** A project whose own switch is still being applied is left to that change. */
	#bulkTargets(active: boolean): string[] {
		return bulkActivationTargets(this.all.filter((p) => !this.toggling[p.id]), active);
	}

	/**
	 * Serves (or stops) every project not already in that state, one at a time. Projects the backend
	 * refuses are named in a warning and keep their state; the list is then reloaded from the backend.
	 */
	async setAllActive(active: boolean): Promise<Outcome<BulkActivation> | null> {
		if (this.bulkToggling) return null;
		const ids = this.#bulkTargets(active);
		if (ids.length === 0) return null;
		this.bulkToggling = true;
		for (const id of ids) this.toggling[id] = true;
		const result = await this.#runner.run(
			() =>
				setEachActive(ids, async (id) => {
					try {
						await this.#backend.commands.setProjectActive({ projectId: id, active });
						this.#patchActive(id, active);
					} finally {
						delete this.toggling[id];
					}
				}),
			{
				success: (r) => (active ? m.projects.bulkActivated(r.changed.length) : m.projects.bulkDeactivated(r.changed.length)),
				warning: (r) => (r.failed.length ? m.projects.bulkFailed(r.failed.map((f) => this.get(f.id)?.name ?? f.id).join(', '), r.changed.length > 0) : null)
			}
		);
		this.bulkToggling = false;
		if (result.ok && result.value.elevationPending) this.#runner.elevationRequired();
		await this.list.load();
		return result;
	}

	#patchActive(id: string, active: boolean) {
		this.list.patch((list) => ({ ...list, projects: list.projects.map((p) => (p.id === id ? { ...p, active } : p)) }));
	}

	/**
	 * The project is created even when its database is not: that is a warning, not a failure, and
	 * so are the project's own warnings (PHP before 7.4 on MySQL 8.4).
	 */
	create(request: CreateProjectRequest) {
		return this.#runner.run(() => this.#backend.commands.createProject(request), {
			success: (r) => (r.databaseSetup?.status === 'created' ? m.projects.createdWithDatabase(r.databaseSetup.name) : m.projects.created),
			warning: (r) => {
				const setup = r.databaseSetup;
				const database = setup && (setup.status === 'package-missing' || setup.status === 'failed') ? m.projects.createdWithoutDatabase(setup.name, setup.message ?? m.common.unknownError) : null;
				const texts = [database, projectWarningsText(r)].filter((t) => t !== null);
				return texts.length ? texts.join(' ') : null;
			}
		});
	}

	update(request: UpdateProjectRequest) {
		return this.#runner.run(() => this.#backend.commands.updateProject(request), { success: m.projects.saved, warning: projectWarningsText });
	}

	setPhp(projectId: string, phpVersion: string) {
		return this.#runner.run(() => this.#backend.commands.setProjectPhp({ projectId, phpVersion }), { warning: projectWarningsText });
	}

	remove(projectId: string) {
		return this.#runner.run(() => this.#backend.commands.deleteProject(projectId), {
			success: m.projects.deleted,
			warning: (r) => (r.orphanedDatabases.length ? m.projects.deletedWithOrphanedDatabases(r.orphanedDatabases.join(', ')) : null)
		});
	}

	relink(request: RelinkProjectRequest) {
		return this.#runner.run(() => this.#backend.commands.relinkProject(request), { success: m.projects.relinked });
	}

	openTerminal(projectId: string) {
		return this.#runner.run(() => this.#backend.commands.openTerminal(projectId));
	}

	openFolder(projectId: string) {
		return this.#runner.run(() => this.#backend.commands.openProjectFolder(projectId), { errors: { validation: m.projects.folderMissing } });
	}

	openInVsCode(projectId: string) {
		return this.#runner.run(() => this.#backend.commands.openInVsCode(projectId), {
			errors: { validation: m.projects.folderMissing, 'package-missing': m.projects.vscodeNotFound }
		});
	}

	export(request: ExportProjectRequest) {
		return this.#runner.run(() => this.#backend.commands.exportProject(request), { success: m.projects.exported });
	}

	import(request: ImportProjectRequest) {
		return this.#runner.run(() => this.#backend.commands.importProject(request), { success: m.projects.imported });
	}

	async start(): Promise<() => void> {
		const stop = await this.#backend.events.onProjectsChanged(() => {
			clearTimeout(this.#timer);
			this.#timer = setTimeout(() => void this.list.load(), EVENT_COALESCE_MS);
		});
		return () => {
			clearTimeout(this.#timer);
			stop();
		};
	}
}
