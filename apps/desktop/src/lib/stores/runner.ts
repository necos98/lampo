import { toAppError, type AppError, type ErrorCode } from '../api/index.js';
import { messages as m } from '../messages/index.js';
import { describeError } from '../services/errors.js';
import type { Notifier } from '../services/notifier.js';

export type Outcome<T> = { ok: true; value: T } | { ok: false; error: AppError };

/**
 * Runs mutating commands: normalises rejections, tells the user, and routes the
 * "needs administrator rights" case to the elevation flow instead of treating it as a failure.
 */
export class CommandRunner {
	readonly #notifier: Notifier;
	readonly #onElevationRequired: () => void;

	constructor(notifier: Notifier, onElevationRequired: () => void) {
		this.#notifier = notifier;
		this.#onElevationRequired = onElevationRequired;
	}

	/**
	 * `errors` replaces the generic message for the given codes with operation-specific text.
	 * `success` may depend on the result; `warning` (when it returns text) replaces the success
	 * toast for a result that succeeded only in part.
	 */
	async run<T>(
		op: () => Promise<T>,
		opts: { success?: string | ((value: T) => string); warning?: (value: T) => string | null; quiet?: boolean; errors?: Partial<Record<ErrorCode, string>> } = {}
	): Promise<Outcome<T>> {
		try {
			const value = await op();
			const warning = opts.warning?.(value) ?? null;
			const success = typeof opts.success === 'function' ? opts.success(value) : opts.success;
			if (warning) this.#notifier.warning(warning);
			else if (success) this.#notifier.success(success);
			return { ok: true, value };
		} catch (e) {
			const error = toAppError(e);
			if (error.code === 'elevation-required') {
				this.#onElevationRequired();
				this.#notifier.info(m.elevation.savedPending);
			} else if (error.code !== 'cancelled' && !opts.quiet) {
				this.#notifier.error(opts.errors?.[error.code] ?? describeError(error));
			}
			return { ok: false, error };
		}
	}
}
