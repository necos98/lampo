import { toAppError, type AppError, type ErrorCode } from '../api/index.js';
import { messages as m } from '../messages/index.js';
import { describeError } from '../services/errors.js';
import { portConflictHref, serviceLogsHref } from '../services/links.js';
import type { Notifier } from '../services/notifier.js';
import { errorNoticeActions, type ErrorActionLinks, type ErrorRecovery } from './errorActions.js';

export type Outcome<T> = { ok: true; value: T } | { ok: false; error: AppError };

export interface RunOptions<T> extends ErrorRecovery {
	success?: string | ((value: T) => string);
	warning?: (value: T) => string | null;
	quiet?: boolean;
	errors?: Partial<Record<ErrorCode, string>>;
}

const LINKS: ErrorActionLinks = { serviceLogs: serviceLogsHref, portConflict: portConflictHref };

/**
 * Runs mutating commands: normalises rejections, tells the user, and routes the
 * "needs administrator rights" case to the elevation flow instead of treating it as a failure.
 */
export class CommandRunner {
	readonly #notifier: Notifier;
	readonly #onElevationRequired: () => void;
	readonly #navigate: (href: string) => void;

	constructor(notifier: Notifier, onElevationRequired: () => void, navigate: (href: string) => void) {
		this.#notifier = notifier;
		this.#onElevationRequired = onElevationRequired;
		this.#navigate = navigate;
	}

	/**
	 * `errors` replaces the generic message for the given codes with operation-specific text.
	 * `success` may depend on the result; `warning` (when it returns text) replaces the success
	 * toast for a result that succeeded only in part. `retry` and `serviceLogs` add «Riprova» and
	 * «Vedi log» to the error toast; a port conflict always links to the services page.
	 */
	async run<T>(op: () => Promise<T>, opts: RunOptions<T> = {}): Promise<Outcome<T>> {
		try {
			const value = await op();
			const warning = opts.warning?.(value) ?? null;
			const success = typeof opts.success === 'function' ? opts.success(value) : opts.success;
			if (warning) this.#notifier.warning(warning);
			else if (success) this.#notifier.success(success);
			return { ok: true, value };
		} catch (e) {
			const error = toAppError(e);
			if (error.code === 'elevation-required') this.elevationRequired();
			else if (error.code !== 'cancelled' && !opts.quiet) {
				const actions = errorNoticeActions(error, opts, m.common.errorActions, LINKS, this.#navigate);
				this.#notifier.error(opts.errors?.[error.code] ?? describeError(error), actions);
			}
			return { ok: false, error };
		}
	}

	/** A change was saved but waits for the hosts file: hands it to the elevation flow and says so. */
	elevationRequired(): void {
		this.#onElevationRequired();
		this.#notifier.info(m.elevation.savedPending);
	}
}
