import type { Project, ProjectEnvironment, SiteAddress, TlsMode, UpdateProjectRequest } from '../api/index.js';

const plain = <V>(v: V): V => JSON.parse(JSON.stringify(v)) as V;

/**
 * Editable copy of a project. Sections bind to its fields; `dirty` compares with the last saved
 * snapshot, `toRequest()` builds the update command payload.
 */
export class ProjectDraft {
	projectId: string;
	name = $state('');
	addresses = $state<SiteAddress[]>([]);
	environment = $state<ProjectEnvironment>({
		phpVersion: '',
		extensions: [],
		addons: [],
		phpIni: {},
		publicDir: '',
		services: [],
		apacheDirectives: ''
	});
	databaseName = $state<string | null>(null);
	tls = $state<TlsMode>('disabled');

	#baseline = '';

	constructor(project: Project) {
		this.projectId = project.id;
		this.reset(project);
	}

	/** Loads `project` as both the draft and the new "saved" baseline. */
	reset(project: Project): void {
		this.name = project.name;
		this.addresses = plain(project.addresses);
		this.environment = plain(project.environment);
		this.databaseName = project.databaseName;
		this.tls = project.tls;
		this.#baseline = this.#serialise();
	}

	#serialise(): string {
		return JSON.stringify([this.name, this.addresses, this.environment, this.databaseName, this.tls]);
	}

	get dirty(): boolean {
		return this.#serialise() !== this.#baseline;
	}

	get valid(): boolean {
		return this.name.trim().length > 0 && this.addresses.length > 0 && this.addresses.every(isAddressFilled);
	}

	toRequest(): UpdateProjectRequest {
		return {
			projectId: this.projectId,
			name: this.name.trim(),
			addresses: plain(this.addresses),
			environment: plain(this.environment),
			databaseName: this.databaseName && this.databaseName.trim() ? this.databaseName.trim() : null,
			tls: this.tls
		};
	}
}

export function isAddressFilled(a: SiteAddress): boolean {
	return a.kind === 'loopback-port' ? Number.isInteger(a.port) && a.port > 0 && a.port < 65536 : a.host.trim().length > 0;
}
