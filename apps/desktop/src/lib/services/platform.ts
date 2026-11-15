/**
 * OS affordances: native pickers and the system browser go through the backend's `Platform`
 * (Tauri plugins in the app, prompts in the mock); the clipboard is a browser API.
 */
import { backend } from '../backend/gateway.js';
import type { PickOptions } from '../backend/ports.js';

export function openExternal(url: string): void {
	void backend.platform.openExternal(url).catch(() => window.open(url, '_blank', 'noopener,noreferrer'));
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
