import type { Project, ServiceStatus } from '../api/index.js';
import type { Messages } from '../messages/index.js';
import type { NavId } from '../services/navigation.js';

/** Sections of the palette, in the order it lists them. */
export type PaletteGroup = 'general' | 'projects' | 'services';

export interface PaletteCommand {
	/** Unique among the commands, stable across renders (a keyed `#each`). */
	id: string;
	group: PaletteGroup;
	label: string;
	/** Extra text the search matches without showing it (a project's id, a service's name). */
	keywords?: string;
	run: () => void;
}

/** What the commands act on: the app's sections (sidebar order), its projects and services. */
export interface PaletteSources {
	sections: readonly { id: NavId; href: string }[];
	projects: readonly Pick<Project, 'id' | 'name' | 'active' | 'folderExists'>[];
	services: readonly Pick<ServiceStatus, 'service' | 'label' | 'state'>[];
}

/** The app operations behind the commands: the same ones the pages use. */
export interface PaletteActions {
	navigate(href: string): void;
	newProject(): void;
	setProjectActive(id: string, active: boolean): void;
	/** Starts or stops `services`; an empty list acts on every planned service. */
	actOnServices(action: 'start' | 'stop', services: string[]): void;
	showHelp(): void;
}

export type PaletteLabels = Messages['palette'];

const projectHref = (id: string) => `/projects/${encodeURIComponent(id)}`;

/** A running or starting service is stopped; any other state is started. */
const isUp = (state: ServiceStatus['state']) => state === 'running' || state === 'starting';

function generalCommands(sections: PaletteSources['sections'], actions: PaletteActions, t: PaletteLabels): PaletteCommand[] {
	return [
		{ id: 'new-project', group: 'general', label: t.newProject, run: () => actions.newProject() },
		...sections.map((item) => ({ id: `nav:${item.id}`, group: 'general' as const, label: t.goTo[item.id], run: () => actions.navigate(item.href) })),
		{ id: 'help', group: 'general', label: t.showHelp, run: () => actions.showHelp() }
	];
}

function projectCommands(projects: PaletteSources['projects'], actions: PaletteActions, t: PaletteLabels): PaletteCommand[] {
	return projects.flatMap((p) => {
		const open: PaletteCommand = { id: `project:${p.id}`, group: 'projects', label: t.openProject(p.name), keywords: p.id, run: () => actions.navigate(projectHref(p.id)) };
		// A project whose folder is missing cannot be served: its switch is disabled too.
		if (!p.folderExists) return [open];
		const toggle: PaletteCommand = {
			id: `project-active:${p.id}`,
			group: 'projects',
			label: p.active ? t.deactivateProject(p.name) : t.activateProject(p.name),
			keywords: p.id,
			run: () => actions.setProjectActive(p.id, !p.active)
		};
		return [open, toggle];
	});
}

function serviceCommands(services: PaletteSources['services'], actions: PaletteActions, t: PaletteLabels): PaletteCommand[] {
	if (services.length === 0) return [];
	const each = services.map((s): PaletteCommand => {
		const label = s.label || s.service;
		const up = isUp(s.state);
		return {
			id: `service:${s.service}`,
			group: 'services',
			label: up ? t.stopService(label) : t.startService(label),
			keywords: s.service,
			run: () => actions.actOnServices(up ? 'stop' : 'start', [s.service])
		};
	});
	return [
		{ id: 'services:start', group: 'services', label: t.startAll, run: () => actions.actOnServices('start', []) },
		{ id: 'services:stop', group: 'services', label: t.stopAll, run: () => actions.actOnServices('stop', []) },
		...each
	];
}

/** Every command of the palette, grouped: general first, then projects, then services. */
export function buildCommands(sources: PaletteSources, actions: PaletteActions, labels: PaletteLabels): PaletteCommand[] {
	return [...generalCommands(sources.sections, actions, labels), ...projectCommands(sources.projects, actions, labels), ...serviceCommands(sources.services, actions, labels)];
}

/**
 * The commands whose label or keywords contain every word of `query` (case-insensitive, any
 * order), in their original order. A blank query keeps them all.
 */
export function filterCommands(commands: readonly PaletteCommand[], query: string): PaletteCommand[] {
	const words = query.toLocaleLowerCase().split(/\s+/).filter(Boolean);
	if (words.length === 0) return [...commands];
	return commands.filter((c) => {
		const text = `${c.label} ${c.keywords ?? ''}`.toLocaleLowerCase();
		return words.every((w) => text.includes(w));
	});
}

/** The highlighted row after moving by `delta` among `count` rows, wrapping at both ends. */
export function moveSelection(index: number, delta: number, count: number): number {
	if (count <= 0) return 0;
	return (((index + delta) % count) + count) % count;
}
