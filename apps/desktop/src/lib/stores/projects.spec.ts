import { beforeAll, describe, expect, it, vi } from 'vitest';

import type { AppError, Project, ProjectList } from '../api/index.js';
import { createMockBackend } from '../backend/mock/index.js';
import { messages as m } from '../messages/index.js';
import { deferred, recordingNotifier, testBackend, testRunner } from '../testing/harness.js';
import { ProjectsStore } from './projects.svelte.js';

const REFUSED: AppError = { code: 'config-rejected', message: 'apache refused the config' };
const ELEVATION: AppError = { code: 'elevation-required', message: 'hosts file needs admin rights' };

/** A complete project of the mock, to vary by id, name and state. */
let template: Project;

beforeAll(async () => {
	[template] = (await createMockBackend({ fresh: false }).commands.listProjects()).projects;
});

const project = (id: string, active: boolean, folderExists = true): Project => ({ ...template, id, name: id.toUpperCase(), active, folderExists });

/** `shop` and `blog` are inactive, `api` is active, `old` has lost its folder. */
const initial = () => [project('shop', false), project('blog', false), project('api', true), project('old', false, false)];

/**
 * A store over `initial()`, loaded. `setOne` answers each project's switch (default: saved); the
 * list the backend returns follows the switches it saved.
 */
async function loadedStore(setOne: (id: string, active: boolean) => Promise<void> = async () => {}) {
	let projects = initial();
	const calls: string[] = [];
	const notifier = recordingNotifier();
	const elevation = vi.fn();
	const env = testBackend({
		listProjects: async (): Promise<ProjectList> => ({ projects: structuredClone(projects), unreadable: [] }),
		setProjectActive: async ({ projectId, active }) => {
			calls.push(projectId);
			await setOne(projectId, active);
			projects = projects.map((p) => (p.id === projectId ? { ...p, active } : p));
			return projects.find((p) => p.id === projectId)!;
		}
	});
	const store = new ProjectsStore(env.backend, testRunner(notifier, elevation));
	await store.load();
	return { store, calls, notifier, elevation };
}

describe('ProjectsStore bulk activation targets', () => {
	it('counts the projects not in the requested state, never a missing folder', async () => {
		const { store } = await loadedStore();

		expect(store.bulkTargetCount(true)).toBe(2);
		expect(store.bulkTargetCount(false)).toBe(1);
	});

	it('leaves out a project whose own switch is still being applied', async () => {
		const pending = deferred<void>();
		const { store } = await loadedStore(() => pending.promise);

		const switching = store.setActive('shop', true);

		expect(store.bulkTargetCount(true)).toBe(1);
		pending.resolve();
		await switching;
	});
});

describe('ProjectsStore.setAllActive', () => {
	it('switches every target one at a time, flagging each until its change is applied', async () => {
		const gates = { shop: deferred<void>(), blog: deferred<void>() };
		const { store, calls, notifier } = await loadedStore((id) => gates[id as keyof typeof gates].promise);

		const running = store.setAllActive(true);

		expect(store.bulkToggling).toBe(true);
		expect(store.toggling).toEqual({ shop: true, blog: true });
		expect(calls).toEqual(['shop']);

		gates.shop.resolve();
		await vi.waitFor(() => expect(calls).toEqual(['shop', 'blog']));
		expect(store.toggling).toEqual({ blog: true });
		expect(store.get('shop')?.active).toBe(true);

		gates.blog.resolve();
		const result = await running;

		expect(result?.ok && result.value.changed).toEqual(['shop', 'blog']);
		expect(store.toggling).toEqual({});
		expect(store.bulkToggling).toBe(false);
		expect(notifier.successes).toEqual([m.projects.bulkActivated(2)]);
		expect(notifier.warnings).toEqual([]);
	});

	it('refuses to start again while a bulk switch runs', async () => {
		const pending = deferred<void>();
		const { store, calls } = await loadedStore(() => pending.promise);

		const first = store.setAllActive(false);

		expect(await store.setAllActive(false)).toBeNull();
		expect(await store.setAllActive(true)).toBeNull();
		pending.resolve();
		await first;
		expect(calls).toEqual(['api']);
	});

	it('does nothing when no project would change', async () => {
		const { store, calls, notifier } = await loadedStore();
		await store.setAllActive(false);

		expect(await store.setAllActive(false)).toBeNull();
		expect(calls).toEqual(['api']);
		expect(notifier.successes).toEqual([m.projects.bulkDeactivated(1)]);
	});

	it('names the refused projects and says the others were updated', async () => {
		const { store, notifier } = await loadedStore(async (id) => {
			if (id === 'blog') throw REFUSED;
		});

		await store.setAllActive(true);

		expect(notifier.warnings).toEqual([m.projects.bulkFailed('BLOG', true)]);
		expect(notifier.successes).toEqual([]);
		expect(store.get('shop')?.active).toBe(true);
		expect(store.get('blog')?.active).toBe(false);
	});

	it('says no project was updated when every one was refused', async () => {
		const { store, notifier } = await loadedStore(async () => {
			throw REFUSED;
		});

		await store.setAllActive(true);

		expect(notifier.warnings).toEqual([m.projects.bulkFailed('SHOP, BLOG', false)]);
		expect(store.toggling).toEqual({});
	});

	it('hands a change waiting for the hosts file to the elevation flow, once', async () => {
		const { store, elevation, notifier } = await loadedStore(async () => {
			throw ELEVATION;
		});

		const result = await store.setAllActive(true);

		expect(result?.ok && result.value).toEqual({ changed: ['shop', 'blog'], failed: [], elevationPending: true });
		expect(elevation).toHaveBeenCalledTimes(1);
		expect(notifier.infos).toEqual([m.elevation.savedPending]);
	});
});

describe('ProjectsStore bulk failure text', () => {
	it('tells a partial refusal from a complete one', () => {
		expect(m.projects.bulkFailed('blog', true)).toBe('Stato non cambiato per: blog. Gli altri progetti sono stati aggiornati.');
		expect(m.projects.bulkFailed('shop, blog', false)).toBe('Stato non cambiato per: shop, blog. Nessun progetto è stato aggiornato.');
	});
});
