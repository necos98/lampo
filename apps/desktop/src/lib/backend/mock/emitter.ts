import type { EventBus } from '../ports.js';
import type * as T from '../../api/types.js';

type Handler<P> = (payload: P) => void;

/** In-memory event bus mirroring the Tauri event contract. */
export class MockEmitter {
	readonly #handlers = new Map<string, Set<Handler<never>>>();

	emit<P>(name: string, payload: P): void {
		for (const h of this.#handlers.get(name) ?? []) (h as Handler<P>)(payload);
	}

	subscribe<P>(name: string, handler: Handler<P>): Promise<() => void> {
		const set = this.#handlers.get(name) ?? new Set();
		set.add(handler as Handler<never>);
		this.#handlers.set(name, set);
		return Promise.resolve(() => void set.delete(handler as Handler<never>));
	}

	bus(): EventBus {
		const sub =
			<P>(name: string) =>
			(h: Handler<P>) =>
				this.subscribe(name, h);
		return {
			onTaskProgress: sub<T.TaskProgressPayload>('task-progress'),
			onTaskFinished: sub<T.TaskFinishedPayload>('task-finished'),
			onServiceStatus: sub<T.ServiceStatusPayload>('service-status'),
			onServiceLog: sub<T.ServiceLogPayload>('service-log'),
			onProjectsChanged: sub<T.ProjectsChangedPayload>('projects-changed'),
			onWorktreesChanged: sub<T.WorktreesChangedPayload>('worktrees-changed'),
			onSettingsChanged: (h: () => void) => this.subscribe<void>('settings-changed', h),
			onPackagesChanged: (h: () => void) => this.subscribe<void>('packages-changed', h),
			onServiceAutoRestart: sub<T.ServiceAutoRestartPayload>('service-auto-restart')
		};
	}
}
