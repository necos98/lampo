import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import type { CommandRunner } from './runner.js';

/**
 * Pending hosts-file change that needs administrator rights. The backend never prompts on its
 * own: it reports (an `elevation-required` error or `hostsFileInSync = false`) and the user
 * triggers `applyEnvironment` explicitly.
 */
export class ElevationStore {
	pending = $state(false);
	applying = $state(false);

	readonly #backend: Backend;
	readonly #runner: CommandRunner;

	constructor(backend: Backend, runner: CommandRunner) {
		this.#backend = backend;
		this.#runner = runner;
	}

	#timer: ReturnType<typeof setTimeout> | undefined;

	/** Addresses are what the hosts file mirrors: re-check (debounced) when projects change. */
	async start(): Promise<() => void> {
		const stop = await this.#backend.events.onProjectsChanged(() => {
			clearTimeout(this.#timer);
			this.#timer = setTimeout(() => void this.refresh(), 600);
		});
		return () => {
			clearTimeout(this.#timer);
			stop();
		};
	}

	flag(): void {
		this.pending = true;
	}

	/**
	 * Re-derives the state from the backend (window re-created, hosts edited elsewhere). Asks only
	 * for the hosts-file comparison: the full doctor also probes ports, PATH and runtimes.
	 */
	async refresh(): Promise<void> {
		try {
			this.pending = !(await this.#backend.commands.hostsInSync());
		} catch {
			// Keep the last known state: an unreadable hosts file says nothing about its content.
		}
	}

	async apply(): Promise<void> {
		if (this.applying) return;
		this.applying = true;
		const result = await this.#runner.run(() => this.#backend.commands.applyEnvironment(), { success: m.elevation.applied, retry: () => this.apply() });
		this.applying = false;
		if (result.ok) this.pending = false;
	}
}
