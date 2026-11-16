/**
 * Typed event subscriptions. Names mirror `lampo_application::events::names` (single source of
 * truth on the Rust side). Each `on…` returns an unsubscribe function.
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import type * as T from './types.js';

export const EVENTS = {
	taskProgress: 'lampo://task-progress',
	taskFinished: 'lampo://task-finished',
	serviceStatus: 'lampo://service-status',
	serviceLog: 'lampo://service-log',
	projectsChanged: 'lampo://projects-changed',
	worktreesChanged: 'lampo://worktrees-changed',
	settingsChanged: 'lampo://settings-changed',
	packagesChanged: 'lampo://packages-changed',
	serviceAutoRestart: 'lampo://service-auto-restart'
} as const;

export type EventName = (typeof EVENTS)[keyof typeof EVENTS];

/** Payload type of each event. */
export interface EventPayloads {
	'lampo://task-progress': T.TaskProgressPayload;
	'lampo://task-finished': T.TaskFinishedPayload;
	'lampo://service-status': T.ServiceStatusPayload;
	'lampo://service-log': T.ServiceLogPayload;
	'lampo://projects-changed': T.ProjectsChangedPayload;
	'lampo://worktrees-changed': T.WorktreesChangedPayload;
	'lampo://settings-changed': null;
	'lampo://packages-changed': null;
	'lampo://service-auto-restart': T.ServiceAutoRestartPayload;
}

export function on<E extends EventName>(event: E, handler: (payload: EventPayloads[E]) => void): Promise<UnlistenFn> {
	return listen<EventPayloads[E]>(event, (e) => handler(e.payload));
}

export const onTaskProgress = (h: (p: T.TaskProgressPayload) => void) => on(EVENTS.taskProgress, h);
export const onTaskFinished = (h: (p: T.TaskFinishedPayload) => void) => on(EVENTS.taskFinished, h);
export const onServiceStatus = (h: (p: T.ServiceStatusPayload) => void) => on(EVENTS.serviceStatus, h);
export const onServiceLog = (h: (p: T.ServiceLogPayload) => void) => on(EVENTS.serviceLog, h);
export const onProjectsChanged = (h: (p: T.ProjectsChangedPayload) => void) => on(EVENTS.projectsChanged, h);
export const onWorktreesChanged = (h: (p: T.WorktreesChangedPayload) => void) => on(EVENTS.worktreesChanged, h);
export const onSettingsChanged = (h: () => void) => on(EVENTS.settingsChanged, () => h());
export const onPackagesChanged = (h: () => void) => on(EVENTS.packagesChanged, () => h());
export const onServiceAutoRestart = (h: (p: T.ServiceAutoRestartPayload) => void) => on(EVENTS.serviceAutoRestart, h);
