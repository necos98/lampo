import type { Project, ProjectEnvironment, SiteAddress, TlsMode, UpdateProjectRequest } from '../api/index.js';
import { AddressList, type AddressRow } from '../projects/addressList.js';
import { DATABASE_KIND, usesService } from '../projects/services.js';
import { Baseline } from './baseline.js';

const plain = <V>(v: V): V => JSON.parse(JSON.stringify(v)) as V;

type EditableFields = Pick<Project, 'name' | 'addresses' | 'environment' | 'databaseName' | 'tls'>;

/** The editable fields of a project, in the order the baseline compares them. */
const editable = (p: EditableFields) => [p.name, p.addresses, p.environment, p.databaseName, p.tls];

/**
 * Editable copy of a project. Sections bind to its fields; `dirty` compares with the last saved
 * snapshot, `toRequest()` builds the update command payload.
 */
export class ProjectDraft {
	projectId: string;
	name = $state('');
	#addressRows = $state<AddressRow[]>([]);
	readonly addresses = new AddressList(this.#addressRows);
	environment = $state<ProjectEnvironment>({
		phpVersion: '',
		extensions: [],
		addons: [],
		phpIni: {},
		publicDir: '',
		services: [],
		apacheDirectives: '',
		envVars: {}
	});
	databaseName = $state<string | null>(null);
	tls = $state<TlsMode>('disabled');

	#baseline = new Baseline();

	constructor(project: Project) {
		this.projectId = project.id;
		this.reset(project);
	}

	/** Loads `project` as both the draft and the new "saved" baseline. */
	reset(project: Project): void {
		this.name = project.name;
		this.addresses.load(plain(project.addresses));
		this.environment = plain(project.environment);
		this.databaseName = project.databaseName;
		this.tls = project.tls;
		this.#baseline.set(editable(this.#fields()));
	}

	/** Takes `project` as the new baseline but keeps the edits, so saving overwrites it. */
	rebase(project: Project): void {
		this.#baseline.set(editable(project));
	}

	/** True when `project` (from the backend) differs from what this draft was loaded from. */
	isStale(project: Project): boolean {
		return !this.#baseline.matches(editable(project));
	}

	get dirty(): boolean {
		return !this.#baseline.matches(editable(this.#fields()));
	}

	get valid(): boolean {
		return this.name.trim().length > 0 && this.addresses.rows.length > 0 && this.addresses.values.every(isAddressFilled);
	}

	/** The database name is only sent while the project uses MySQL (the backend refuses it otherwise). */
	toRequest(): UpdateProjectRequest {
		const databaseName = usesService(this.environment, DATABASE_KIND) ? this.databaseName?.trim() : null;
		return {
			projectId: this.projectId,
			name: this.name.trim(),
			addresses: plain(this.addresses.values),
			environment: plain(this.environment),
			databaseName: databaseName || null,
			tls: this.tls
		};
	}

	#fields(): EditableFields {
		return { name: this.name, addresses: this.addresses.values, environment: this.environment, databaseName: this.databaseName, tls: this.tls };
	}
}

export function isAddressFilled(a: SiteAddress): boolean {
	return a.kind === 'loopback-port' ? Number.isInteger(a.port) && a.port > 0 && a.port < 65536 : a.host.trim().length > 0;
}
