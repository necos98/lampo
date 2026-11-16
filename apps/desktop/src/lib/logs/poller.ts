/**
 * Runs `tick` every `intervalMs`, one tick at a time: the next one is scheduled only after the
 * previous has settled, so a slow backend never stacks calls. While hidden it stops scheduling;
 * showing it again ticks at once, so the reader sees fresh lines without waiting an interval.
 * `tick` handles its own errors: a rejection is left unhandled (so it is reported) and polling
 * goes on.
 */
export class Poller {
	readonly #tick: () => Promise<void>;
	readonly #intervalMs: number;
	#timer: ReturnType<typeof setTimeout> | undefined;
	/** Bumped by `start`/`stop`, so a tick still running from an older chain schedules nothing. */
	#chain = 0;
	#running = false;
	#hidden = false;
	#inFlight = false;

	constructor(tick: () => Promise<void>, intervalMs: number) {
		this.#tick = tick;
		this.#intervalMs = intervalMs;
	}

	get scheduled(): boolean {
		return this.#timer !== undefined;
	}

	start(): void {
		this.stop();
		this.#running = true;
		this.#schedule(this.#chain);
	}

	stop(): void {
		this.#chain++;
		this.#running = false;
		this.#inFlight = false;
		this.#cancel();
	}

	setHidden(hidden: boolean): void {
		if (hidden === this.#hidden) return;
		this.#hidden = hidden;
		if (hidden) {
			this.#cancel();
		} else if (this.#running && !this.#inFlight) {
			this.#cancel();
			void this.#run(this.#chain);
		}
	}

	#schedule(chain: number): void {
		if (chain !== this.#chain || !this.#running || this.#hidden) return;
		this.#timer = setTimeout(() => {
			this.#timer = undefined;
			void this.#run(chain);
		}, this.#intervalMs);
	}

	#cancel(): void {
		clearTimeout(this.#timer);
		this.#timer = undefined;
	}

	async #run(chain: number): Promise<void> {
		this.#inFlight = true;
		try {
			await this.#tick();
		} finally {
			if (chain === this.#chain) {
				this.#inFlight = false;
				this.#schedule(chain);
			}
		}
	}
}
