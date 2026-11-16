import type { AutoRestartStep, ServiceAutoRestartPayload } from '../api/types.js';
import type { Notifier } from '../services/notifier.js';

/** The texts of the notices (`messages.services.autoRestart`). */
export interface AutoRestartTexts {
	scheduled: (label: string, attempt: number, maxAttempts: number, seconds: number) => string;
	restarted: (label: string) => string;
	gaveUp: (label: string, attempts: number) => string;
}

/** The subscription this store needs from the event bus. */
export interface AutoRestartEvents {
	onServiceAutoRestart(handler: (payload: ServiceAutoRestartPayload) => void): Promise<() => void>;
}

const MS_PER_SECOND = 1000;

/**
 * Tells the user about every step of the automatic restart of a crashed service: a warning when
 * an attempt is scheduled, a success when the service runs again, an error (with the last
 * failure) when Lampo gives up.
 */
export class AutoRestartNotices {
	readonly #events: AutoRestartEvents;
	readonly #notifier: Notifier;
	readonly #texts: AutoRestartTexts;
	readonly #labelOf: (service: string) => string;

	constructor(events: AutoRestartEvents, notifier: Notifier, texts: AutoRestartTexts, labelOf: (service: string) => string) {
		this.#events = events;
		this.#notifier = notifier;
		this.#texts = texts;
		this.#labelOf = labelOf;
	}

	start(): Promise<() => void> {
		return this.#events.onServiceAutoRestart(({ service, step }) => this.#notify(this.#labelOf(service), step));
	}

	#notify(label: string, step: AutoRestartStep): void {
		const t = this.#texts;
		switch (step.type) {
			case 'scheduled':
				this.#notifier.warning(t.scheduled(label, step.attempt, step.maxAttempts, Math.round(step.delayMs / MS_PER_SECOND)));
				break;
			case 'restarted':
				this.#notifier.success(t.restarted(label));
				break;
			case 'gave-up':
				this.#notifier.error(t.gaveUp(label, step.attempts), step.message);
				break;
		}
	}
}
