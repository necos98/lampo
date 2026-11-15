import type { Backend } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { describeError } from '../services/errors.js';
import type { Notifier } from '../services/notifier.js';

/**
 * Pending hosts-file change that needs administrator rights. The backend never prompts on its
 * own: it reports (an `elevation-required` error or `hostsFileInSync = false`) and the user
 * triggers `applyEnvironment` explicitly.
 */
export class ElevationStore {
	pending = $state(false);
	applying = $state(false);

	readonly #backend: Backend;
	readonly #notifier: Notifier;

	constructor(backend: Backend, notifier: Notifier) {
		this.#backend = backend;
		this.#notifier = notifier;
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

	/** Re-derives the state from the backend (window re-created, hosts edited elsewhere). */
	async refresh(): Promise<void> {
		try {
			const report = await this.#backend.commands.runDoctor();
			this.pending = !report.hostsFileInSync;
		} catch {
			// Keep the last known state: a failed doctor run says nothing about the hosts file.
		}
	}

	async apply(): Promise<void> {
		if (this.applying) return;
		this.applying = true;
		try {
			await this.#backend.commands.applyEnvironment();
			this.pending = false;
			this.#notifier.success(m.elevation.applied);
		} catch (e) {
			this.#notifier.error(describeError(e));
		} finally {
			this.applying = false;
		}
	}
}
