import type { AppError } from '../api/index.js';
import type { NoticeActions, NotifierAction } from '../services/notifier.js';

/** The labels of the buttons (`messages.common`). */
export interface ErrorActionTexts {
	retry: string;
	viewLogs: string;
	viewConflict: string;
}

/** Where the links of an error notice lead. */
export interface ErrorActionLinks {
	serviceLogs(service: string | null): string;
	portConflict(port: number): string;
}

/** What a failed operation offers the user besides the message. */
export interface ErrorRecovery {
	/** Runs the failed operation again («Riprova»). */
	retry?: () => unknown;
	/**
	 * The failure concerns services: «Vedi log» opens the log of the service this returns. Read on
	 * click, when the status events naming the failed service have arrived; `null` lets the logs
	 * page pick.
	 */
	serviceLogs?: () => string | null;
}

/** `details.port` of a `port-in-use` error, if the backend named it. */
export function conflictPort(error: AppError): number | null {
	if (error.code !== 'port-in-use') return null;
	const port = (error.details as { port?: unknown } | undefined)?.port;
	return typeof port === 'number' ? port : null;
}

/**
 * The buttons of an error notice: «Riprova» first when the operation can be repeated, then the
 * link explaining the failure (the port conflict, which says more than any log, or the service's
 * log).
 */
export function errorNoticeActions(error: AppError, recovery: ErrorRecovery, texts: ErrorActionTexts, links: ErrorActionLinks, navigate: (href: string) => void): NoticeActions {
	const link = explanationLink(error, recovery, texts, links, navigate);
	const { retry } = recovery;
	if (!retry) return { action: link };
	return { action: { label: texts.retry, run: () => void retry() }, secondaryAction: link };
}

function explanationLink(error: AppError, { serviceLogs }: ErrorRecovery, texts: ErrorActionTexts, links: ErrorActionLinks, navigate: (href: string) => void): NotifierAction | undefined {
	const port = conflictPort(error);
	if (port !== null) return { label: texts.viewConflict, run: () => navigate(links.portConflict(port)) };
	if (serviceLogs) return { label: texts.viewLogs, run: () => navigate(links.serviceLogs(serviceLogs())) };
	return undefined;
}
