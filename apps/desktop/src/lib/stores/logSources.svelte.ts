import type { LogSource } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { Resource } from './resource.svelte.js';

export type LogGroup = 'services' | 'access' | 'error' | 'php';

export function groupOf(source: LogSource): LogGroup {
	if (source.id.startsWith('access:')) return 'access';
	if (source.id.startsWith('error:')) return 'error';
	if (source.id.startsWith('php:')) return 'php';
	return 'services';
}

export class LogSourcesStore {
	readonly list: Resource<LogSource[]>;

	constructor(backend: Backend) {
		this.list = new Resource(() => backend.commands.listLogSources(null));
	}

	load = () => this.list.load();

	grouped(): { group: LogGroup; sources: LogSource[] }[] {
		const order: LogGroup[] = ['services', 'error', 'access', 'php'];
		const all = this.list.data ?? [];
		return order.map((group) => ({ group, sources: all.filter((s) => groupOf(s) === group) })).filter((g) => g.sources.length);
	}
}
