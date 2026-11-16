/**
 * The only seam between the GUI and the backend. Stores depend on `Backend`, never on `invoke`
 * or on event names. Two adapters implement it: `tauri.ts` (real) and `mock/` (browser dev).
 */
import type * as commands from '../api/commands.js';
import type * as events from '../api/events.js';

/** Every typed command of the contract. */
export type Commands = Omit<typeof commands, 'COMMANDS'>;

/** Every typed event subscription of the contract. */
export type EventBus = Pick<
	typeof events,
	| 'onTaskProgress'
	| 'onTaskFinished'
	| 'onServiceStatus'
	| 'onServiceLog'
	| 'onProjectsChanged'
	| 'onWorktreesChanged'
	| 'onSettingsChanged'
	| 'onPackagesChanged'
	| 'onServiceAutoRestart'
>;

export interface FileFilter {
	/** Shown in the picker (`Configurazione Lampo`). */
	name: string;
	/** Without the dot (`json`). */
	extensions: string[];
}

export interface PickOptions {
	title?: string;
	/** Folder (or file, for saving) the picker starts from. */
	defaultPath?: string;
	filters?: FileFilter[];
}

/** OS affordances outside the command contract: native pickers and the system browser. */
export interface Platform {
	/** Absolute folder path, or `null` when the user cancelled. */
	pickFolder(options?: PickOptions): Promise<string | null>;
	/** Absolute path of an existing file, or `null`. */
	pickFile(options?: PickOptions): Promise<string | null>;
	/** Absolute path to write to, or `null`. */
	pickSaveFile(options?: PickOptions): Promise<string | null>;
	/** Opens an http(s) URL in the default browser. */
	openExternal(url: string): Promise<void>;
}

/** Download progress of an update; `total` is `null` when the server sends no size. */
export type UpdateDownloadEvent = { kind: 'started'; total: number | null } | { kind: 'progress'; bytes: number } | { kind: 'finished' };

/** A newer release found by [`Updater.check`] (signed installer, verified on download). */
export interface AvailableUpdate {
	/** Without the `v` (`0.2.0`). */
	readonly version: string;
	/** Release date (ISO 8601), when the manifest has one. */
	readonly date: string | null;
	/** Release notes, when the manifest has them. */
	readonly notes: string | null;
	/** Downloads the installer and verifies its signature. */
	download(onEvent: (event: UpdateDownloadEvent) => void): Promise<void>;
	/** Starts the downloaded installer. On Windows the app exits right away. */
	install(): Promise<void>;
}

/** In-app updates (`tauri-plugin-updater` + `tauri-plugin-process` in the app). */
export interface Updater {
	/** Version of the running app. */
	currentVersion(): Promise<string>;
	/** `null` when the app is up to date. Rejects without network or release. */
	check(): Promise<AvailableUpdate | null>;
	/** Restarts the app (fallback when the process is still alive after `install`). */
	relaunch(): Promise<void>;
}

export interface Backend {
	/** `true` when the data is simulated. */
	readonly simulated: boolean;
	readonly commands: Commands;
	readonly events: EventBus;
	readonly platform: Platform;
	readonly updater: Updater;
}
