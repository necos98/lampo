import { toast } from 'svelte-sonner';

/** A button on a notification (e.g. "Registra" on "Cartella pronta"). */
export interface NotifierAction {
	label: string;
	run: () => void;
}

/** Thin wrapper so stores do not depend on the toast library. */
export interface Notifier {
	success(message: string, action?: NotifierAction): void;
	info(message: string): void;
	/** Done, but something the user should know about went wrong (e.g. the database was not created). */
	warning(message: string): void;
	error(message: string, description?: string): void;
}

export const notifier: Notifier = {
	success: (message, action) => void toast.success(message, action ? { action: { label: action.label, onClick: () => action.run() } } : undefined),
	info: (message) => void toast.info(message),
	warning: (message) => void toast.warning(message),
	error: (message, description) => void toast.error(message, { description })
};
