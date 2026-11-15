import { toast } from 'svelte-sonner';

/** Thin wrapper so stores do not depend on the toast library. */
export interface Notifier {
	success(message: string): void;
	info(message: string): void;
	/** Done, but something the user should know about went wrong (e.g. the database was not created). */
	warning(message: string): void;
	error(message: string, description?: string): void;
}

export const notifier: Notifier = {
	success: (message) => void toast.success(message),
	info: (message) => void toast.info(message),
	warning: (message) => void toast.warning(message),
	error: (message, description) => void toast.error(message, { description })
};
