import type { ServiceStatus } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { Resource } from './resource.svelte.js';
import type { CommandRunner } from './runner.js';

export type ServiceAction = 'start' | 'stop' | 'restart';

/** Live status of Apache, MySQL instances and Redis, plus start/stop/restart. */
export class ServicesStore {
	readonly list: Resource<ServiceStatus[]>;
	/** The user-triggered action in flight per service. */
	busy = $state<Record<string, ServiceAction>>({});
	/** Last failed action per service, to show port-in-use / config output. */
	failures = $state<Record<string, unknown>>({});

	readonly #backend: Backend;
	readonly #runner: CommandRunner;

	constructor(backend: Backend, runner: CommandRunner) {
		this.#backend = backend;
		this.#runner = runner;
		this.list = new Resource(() => backend.commands.listServices());
	}

	load = () => this.list.load();

	get anyRunning(): boolean {
		return (this.list.data ?? []).some((s) => s.state === 'running' || s.state === 'starting');
	}

	/** Some service has a user-triggered action in flight. */
	get anyBusy(): boolean {
		return Object.keys(this.busy).length > 0;
	}

	/**
	 * Whether `action` may run on `names` now: none has an action in flight, except a start that a
	 * stop gives up. An empty `names` (every service, as `act` reads it) waits for nothing in flight,
	 * as the «tutti» buttons of the services page do.
	 */
	canAct(action: ServiceAction, names: string[]): boolean {
		if (names.length === 0) return !this.anyBusy;
		return names.every((n) => this.busy[n] === undefined || (action === 'stop' && this.busy[n] === 'start'));
	}

	/** An empty `services` array acts on every planned service. A start given up by a stop is no failure. */
	async act(action: ServiceAction, services: string[] = []): Promise<void> {
		const names = services.length ? services : (this.list.data ?? []).map((s) => s.service);
		for (const n of names) this.busy[n] = action;
		const cmd = this.#backend.commands;
		const call = action === 'start' ? cmd.startServices : action === 'stop' ? cmd.stopServices : cmd.restartServices;
		const result = await this.#runner.run(() => call({ services }), {
			success: services.length ? undefined : m.services.done[action],
			// The toast outlives the action: a «Riprova» while the services are busy again does nothing.
			retry: () => (this.canAct(action, services) ? this.act(action, services) : undefined),
			serviceLogs: () => this.#failedAmong(names)
		});
		// A stop that overtook this action (giving up its start) owns the flag now.
		for (const n of names) if (this.busy[n] === action) delete this.busy[n];
		for (const n of names) {
			if (result.ok || result.error.code === 'cancelled') delete this.failures[n];
			else if (services.length) this.failures[n] = result.error;
		}
		await this.list.load();
	}

	/** The service whose log explains a failed action: the only one acted on, else the first that failed. */
	#failedAmong(names: string[]): string | null {
		if (names.length === 1) return names[0];
		return (this.list.data ?? []).find((s) => s.state === 'failed' && names.includes(s.service))?.service ?? null;
	}

	async start(): Promise<() => void> {
		return this.#backend.events.onServiceStatus((p) => {
			this.list.patch((current) => {
				if (!current.some((s) => s.service === p.service)) {
					return [...current, { service: p.service, kind: p.kind, label: p.service, state: p.state, pid: p.pid, port: p.port, message: p.message, required: false, requiredBy: [] }];
				}
				return current.map((s) => (s.service === p.service ? { ...s, state: p.state, pid: p.pid, port: p.port, message: p.message } : s));
			});
		});
	}
}
