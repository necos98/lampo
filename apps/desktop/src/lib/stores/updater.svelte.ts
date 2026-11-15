import type { AvailableUpdate, Backend } from '../backend/ports.js';
import { describeError } from '../services/errors.js';

export type UpdatePhase = 'idle' | 'checking' | 'available' | 'uptodate' | 'downloading' | 'installing' | 'error';

/** Text of an updater failure: plugin errors are plain strings, command errors are `AppError`s. */
function errorText(e: unknown): string {
	if (typeof e === 'string') return e;
	if (e instanceof Error) return e.message;
	return describeError(e);
}

/**
 * In-app updates (signed NSIS installer from the GitHub Releases). A silent check at startup
 * lights the dot on "Impostazioni"; the settings page checks on demand and installs.
 *
 * Install order: download (signature verified) → `prepareUpdate` (Apache, MySQL… stopped
 * cleanly: the plugin's `install()` exits the process at once, skipping the graceful quit) →
 * `install()` → `relaunch()` as fallback if the process is still alive.
 */
export class UpdaterStore {
	phase = $state<UpdatePhase>('idle');
	/** Version of the running app (`null` until known). */
	current = $state<string | null>(null);
	update = $state.raw<AvailableUpdate | null>(null);
	error = $state<string | null>(null);
	/** Download percentage; `null` while the size is unknown. */
	percent = $state<number | null>(null);

	readonly #backend: Backend;
	#silentDone = false;

	constructor(backend: Backend) {
		this.#backend = backend;
	}

	/** An update was found by the last check (dot on the Settings nav entry). */
	get available(): boolean {
		return this.update !== null;
	}

	get busy(): boolean {
		return this.phase === 'checking' || this.phase === 'downloading' || this.phase === 'installing';
	}

	async loadVersion(): Promise<void> {
		if (this.current) return;
		try {
			this.current = await this.#backend.updater.currentVersion();
		} catch {
			// Only a label; outside the app there is none.
		}
	}

	/** Once per window: no network or no release yet are not worth a message. */
	async checkSilently(): Promise<void> {
		if (this.#silentDone) return;
		this.#silentDone = true;
		await this.#check(true);
	}

	check(): Promise<void> {
		return this.#check(false);
	}

	async #check(silent: boolean): Promise<void> {
		if (this.busy) return;
		this.error = null;
		if (!silent) this.phase = 'checking';
		void this.loadVersion();
		try {
			const found = await this.#backend.updater.check();
			this.update = found;
			this.phase = found ? 'available' : 'uptodate';
		} catch (e) {
			if (!silent) {
				this.phase = 'error';
				this.error = errorText(e);
			}
		}
	}

	async install(): Promise<void> {
		const update = this.update;
		if (!update || this.busy) return;
		this.error = null;
		this.percent = null;
		this.phase = 'downloading';
		let total = 0;
		let done = 0;
		let prepared = false;
		try {
			await update.download((e) => {
				if (e.kind === 'started') {
					total = e.total ?? 0;
					this.percent = total > 0 ? 0 : null;
				} else if (e.kind === 'progress') {
					done += e.bytes;
					if (total > 0) this.percent = Math.min(100, Math.round((done / total) * 100));
				} else {
					this.percent = 100;
				}
			});
			this.phase = 'installing';
			try {
				await this.#backend.commands.prepareUpdate();
			} catch (e) {
				// Keep going: the Job Object kills whatever is left when the app exits.
				console.error('prepare_update failed, installing anyway', e);
			}
			prepared = true;
			// On Windows the app exits here, as soon as the installer starts.
			await update.install();
			await this.#backend.updater.relaunch();
		} catch (e) {
			this.phase = 'error';
			this.error = errorText(e);
			if (prepared) {
				// The installer did not start: services back to the configured state.
				this.#backend.commands.cancelUpdate().catch((err) => console.error('cancel_update failed', err));
			}
		}
	}
}
