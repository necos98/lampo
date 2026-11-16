import type { Project, ProjectEnvironment, SiteAddress, TlsMode, UpdateProjectRequest } from '../api/index.js';
import { AddressList, type AddressRow } from '../projects/addressList.js';
import { EnvVarList, type EnvVarRow } from '../projects/envVarList.js';
import { DATABASE_KIND, usesService } from '../projects/services.js';
import { Baseline } from './baseline.js';

const plain = <V>(v: V): V => JSON.parse(JSON.stringify(v)) as V;

type EditableFields = Pick<Project, 'name' | 'addresses' | 'environment' | 'databaseName' | 'tls'>;

/** The environment but its variables, which the draft edits by row (`ProjectDraft.envVars`). */
type EnvironmentFields = Omit<ProjectEnvironment, 'envVars'>;

/** The editable fields of a project, in the order the baseline compares them. */
function editable(p: EditableFields) {
	const { envVars, ...environment } = p.environment;
	return [p.name, p.addresses, environment, envVars, p.databaseName, p.tls];
}

/**
 * Editable copy of a project. Sections bind to its fields; `dirty` compares with the last saved
 * snapshot, `toRequest()` builds the update command payload.
 */
export class ProjectDraft {
	projectId: string;
	name = $state('');
	#addressRows = $state<AddressRow[]>([]);
	readonly addresses = new AddressList(this.#addressRows);
	environment = $state<EnvironmentFields>({
		phpVersion: '',
		extensions: [],
		addons: [],
		phpIni: {},
		publicDir: '',
		services: [],
		apacheDirectives: ''
	});
	#envVarRows = $state<EnvVarRow[]>([]);
	readonly envVars = new EnvVarList(this.#envVarRows);
	databaseName = $state<string | null>(null);
	tls = $state<TlsMode>('disabled');

	#baseline = new Baseline();

	constructor(project: Project) {
		this.projectId = project.id;
		this.reset(project);
	}

	/** Loads `project` as both the draft and the new "saved" baseline. */
	reset(project: Project): void {
		const { envVars, ...environment } = plain(project.environment);
		this.name = project.name;
		this.addresses.load(plain(project.addresses));
		this.environment = environment;
		this.envVars.load(envVars);
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
		return this.name.trim().length > 0 && this.addresses.rows.length > 0 && this.addresses.values.every(isAddressFilled) && this.envVars.valid;
	}

	/** The database name is only sent while the project uses MySQL (the backend refuses it otherwise). */
	toRequest(): UpdateProjectRequest {
		const databaseName = usesService(this.environment, DATABASE_KIND) ? this.databaseName?.trim() : null;
		return {
			projectId: this.projectId,
			name: this.name.trim(),
			addresses: plain(this.addresses.values),
			environment: plain(this.#environment()),
			databaseName: databaseName || null,
			tls: this.tls
		};
	}

	#environment(): ProjectEnvironment {
		return { ...this.environment, envVars: this.envVars.values };
	}

	#fields(): EditableFields {
		return { name: this.name, addresses: this.addresses.values, environment: this.#environment(), databaseName: this.databaseName, tls: this.tls };
	}
}

export function isAddressFilled(a: SiteAddress): boolean {
	return a.kind === 'loopback-port' ? Number.isInteger(a.port) && a.port > 0 && a.port < 65536 : a.host.trim().length > 0;
}
