import { getVersion } from '@tauri-apps/api/app';
import { open, save } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { relaunch } from '@tauri-apps/plugin-process';
import { check, type Update } from '@tauri-apps/plugin-updater';

import * as commands from '../api/commands.js';
import * as events from '../api/events.js';
import type { AvailableUpdate, Backend, Platform, Updater } from './ports.js';

/** Native pickers (`tauri-plugin-dialog`) and the system browser (`tauri-plugin-opener`). */
const platform: Platform = {
	pickFolder: async ({ title, defaultPath } = {}) => {
		const picked = await open({ directory: true, multiple: false, title, defaultPath });
		return typeof picked === 'string' ? picked : null;
	},
	pickFile: async ({ title, defaultPath, filters } = {}) => {
		const picked = await open({ directory: false, multiple: false, title, defaultPath, filters });
		return typeof picked === 'string' ? picked : null;
	},
	pickSaveFile: async ({ title, defaultPath, filters } = {}) => (await save({ title, defaultPath, filters })) ?? null,
	openExternal: (url) => openUrl(url)
};

function wrapUpdate(update: Update): AvailableUpdate {
	return {
		version: update.version,
		date: update.date ?? null,
		notes: update.body?.trim() || null,
		download: (onEvent) =>
			update.download((e) => {
				if (e.event === 'Started') onEvent({ kind: 'started', total: e.data.contentLength ?? null });
				else if (e.event === 'Progress') onEvent({ kind: 'progress', bytes: e.data.chunkLength });
				else onEvent({ kind: 'finished' });
			}),
		install: () => update.install()
	};
}

/** Signed NSIS updates from the GitHub Releases (`plugins.updater` in `tauri.conf.json`). */
const updater: Updater = {
	currentVersion: () => getVersion(),
	check: async () => {
		const update = await check();
		return update ? wrapUpdate(update) : null;
	},
	relaunch: () => relaunch()
};

export function createTauriBackend(): Backend {
	return { simulated: false, commands, events, platform, updater };
}
