/**
 * OS affordances: native pickers and the system browser go through the backend's `Platform`
 * (Tauri plugins in the app, prompts in the mock); the clipboard is a browser API.
 */
import { backend } from '../backend/gateway.js';
import type { PickOptions } from '../backend/ports.js';
import { messages as m } from '../messages/index.js';
import { notifier } from './notifier.js';

/** The only schemes handed to the system browser; anything else (file:, javascript:, …) is refused. */
const EXTERNAL_PROTOCOLS: ReadonlySet<string> = new Set(['http:', 'https:']);

/** `url` parsed, if it is an absolute http(s) URL. */
function parseExternalUrl(url: string): URL | null {
	try {
		const parsed = new URL(url);
		return EXTERNAL_PROTOCOLS.has(parsed.protocol) ? parsed : null;
	} catch {
		return null;
	}
}

/**
 * Opens an http(s) URL in the default browser. Never navigates or opens a window inside the
 * webview: a refused or failed open is reported with a toast.
 */
export function openExternal(url: string): void {
	const parsed = parseExternalUrl(url);
	if (!parsed) {
		notifier.error(m.common.openLinkFailed, m.common.unsupportedLink);
		return;
	}
	void backend.platform.openExternal(parsed.href).catch((err: unknown) => notifier.error(m.common.openLinkFailed, err instanceof Error ? err.message : String(err)));
}

export const pickFolder = (options?: PickOptions) => backend.platform.pickFolder(options);
export const pickFile = (options?: PickOptions) => backend.platform.pickFile(options);
export const pickSaveFile = (options?: PickOptions) => backend.platform.pickSaveFile(options);

export async function copyText(text: string): Promise<boolean> {
	try {
		await navigator.clipboard.writeText(text);
		return true;
	} catch {
		return false;
	}
}
