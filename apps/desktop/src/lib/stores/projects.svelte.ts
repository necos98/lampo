import type {
	AddressSuggestions,
	CreateProjectRequest,
	ExportProjectRequest,
	ImportProjectRequest,
	Project,
	RelinkProjectRequest,
	UpdateProjectRequest
} from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner } from './runner.js';

const EVENT_COALESCE_MS = 150;

export class ProjectsStore {
	readonly list: Resource<Project[]>;
	/** Projects whose active toggle is being applied. */
	toggling = $state<Record<string, boolean>>({});

	readonly #backend: Backend;
	readonly #runner: CommandRunner;
	#timer: ReturnType<typeof setTimeout> | undefined;

	constructor(backend: Backend, runner: CommandRunner) {
		this.#backend = backend;
		this.#runner = runner;
		this.list = new Resource(() => backend.commands.listProjects());
	}

	load = () => this.list.load();

	/** Default addresses and a free loopback port for `id`, from the backend (`null` if it failed). */
	async suggestAddresses(id: string): Promise<AddressSuggestions | null> {
		try {
			return await this.#backend.commands.suggestAddresses(id);
		} catch {
			return null;
		}
	}

	get(id: string): Project | null {
		return this.list.data?.find((p) => p.id === id) ?? null;
	}

	async setActive(id: string, active: boolean) {
		this.toggling[id] = true;
		this.list.patch((all) => all.map((p) => (p.id === id ? { ...p, active } : p)));
		const result = await this.#runner.run(() => this.#backend.commands.setProjectActive({ projectId: id, active }));
		delete this.toggling[id];
		// On failure (or a pending elevation) the backend state is the truth.
		if (!result.ok) await this.list.load();
		return result;
	}

	/** The project is created even when its database is not: that is a warning, not a failure. */
	create(request: CreateProjectRequest) {
		return this.#runner.run(() => this.#backend.commands.createProject(request), {
			success: (r) => (r.databaseSetup?.status === 'created' ? m.projects.createdWithDatabase(r.databaseSetup.name) : m.projects.created),
			warning: (r) => {
				const setup = r.databaseSetup;
				return setup && (setup.status === 'package-missing' || setup.status === 'failed') ? m.projects.createdWithoutDatabase(setup.name, setup.message ?? m.common.unknownError) : null;
			}
		});
	}

	update(request: UpdateProjectRequest) {
		return this.#runner.run(() => this.#backend.commands.updateProject(request), { success: m.projects.saved });
	}

	setPhp(projectId: string, phpVersion: string) {
		return this.#runner.run(() => this.#backend.commands.setProjectPhp({ projectId, phpVersion }));
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
