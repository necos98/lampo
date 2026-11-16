import { toast } from 'svelte-sonner';

/** A button on a notification (e.g. "Registra" on "Cartella pronta"). */
export interface NotifierAction {
	label: string;
	run: () => void;
}

/** The buttons of a notice: the main one and, less prominent, a second one. */
export interface NoticeActions {
	action?: NotifierAction;
	secondaryAction?: NotifierAction;
}

export interface ErrorNotice extends NoticeActions {
	description?: string;
}

/** Thin wrapper so stores do not depend on the toast library. */
export interface Notifier {
	success(message: string, action?: NotifierAction): void;
	info(message: string): void;
	/** Done, but something the user should know about went wrong (e.g. the database was not created). */
	warning(message: string): void;
	error(message: string, notice?: ErrorNotice): void;
}

const toastAction = (action: NotifierAction | undefined) => (action ? { label: action.label, onClick: () => action.run() } : undefined);

export const notifier: Notifier = {
	success: (message, action) => void toast.success(message, { action: toastAction(action) }),
	info: (message) => void toast.info(message),
	warning: (message) => void toast.warning(message),
	error: (message, notice = {}) =>
		void toast.error(message, { description: notice.description, action: toastAction(notice.action), cancel: toastAction(notice.secondaryAction) })
};
