/**
 * Lampo backend API for the GUI.
 *
 *   import { api, events, type Project } from '#lib/api/index.js';
 *   const projects = await api.listProjects();
 *   const stop = await events.onProjectsChanged(() => reload());
 */
export * as api from './commands.js';
export * as events from './events.js';
export * from './errors.js';
export type * from './types.js';

/** `true` inside the Tauri webview, `false` in a plain browser (e.g. `vite dev` without Tauri). */
export function isTauri(): boolean {
	return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
