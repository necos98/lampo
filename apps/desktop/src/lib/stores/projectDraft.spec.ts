import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Project } from '../api/index.js';
import { seedProjects } from '../backend/mock/seed.js';
import { testBackend, testRunner } from '../testing/harness.js';
import { ProjectDraft } from './projectDraft.svelte.js';
import { ProjectsStore } from './projects.svelte.js';

/** The seeded "shop" project: uses MySQL, has two addresses and some env vars. */
function shop(): Project {
	return seedProjects().find((p) => p.id === 'shop')!;
}

/** The seeded "crm-legacy" project: MySQL 5.7. */
function withoutMysql(): Project {
	const project = shop();
	project.environment.services = project.environment.services.filter((s) => s.kind !== 'mysql');
	project.databaseName = null;
	return project;
}

describe('ProjectDraft dirty state', () => {
	it('starts clean, copying the project', () => {
		const project = shop();
		const draft = new ProjectDraft(project);

		expect(draft.dirty).toBe(false);
		expect(draft.name).toBe(project.name);
		expect(draft.projectId).toBe('shop');
		expect(draft.envVars.values).toEqual(project.environment.envVars);
	});

	it('does not alias the project it was built from', () => {
		const project = shop();
		const draft = new ProjectDraft(project);

		draft.environment.extensions.push('gd');

		expect(project.environment.extensions).not.toContain('gd');
	});

	it.each([
		['name', (d: ProjectDraft) => (d.name = 'Renamed')],
		['tls', (d: ProjectDraft) => (d.tls = 'local-ca')],
		['a php setting', (d: ProjectDraft) => (d.environment.phpVersion = '7.4')],
		['an address', (d: ProjectDraft) => d.addresses.setHost(d.addresses.rows[0].id, 'other.test')],
		['an env var', (d: ProjectDraft) => d.envVars.setValue(d.envVars.rows[0].id, 'changed')],
		['the database name', (d: ProjectDraft) => (d.databaseName = 'other_db')],
		['the MySQL credentials', (d: ProjectDraft) => (d.mysqlCredentials = 'root')]
	])('becomes dirty when %s changes, and clean again when it is put back', (_label, edit) => {
		const draft = new ProjectDraft(shop());
		const before = JSON.stringify(draft.toRequest());

		edit(draft);
		expect(draft.dirty).toBe(true);

		draft.reset(shop());
		expect(draft.dirty).toBe(false);
		expect(JSON.stringify(draft.toRequest())).toBe(before);
	});

	it('is clean again when an edit is undone by hand', () => {
		const draft = new ProjectDraft(shop());

		draft.name = 'Renamed';
		draft.name = 'Shop';

		expect(draft.dirty).toBe(false);
	});
});

describe('ProjectDraft baseline', () => {
	it('isStale() is false for the project it was loaded from and true once the backend changed it', () => {
		const draft = new ProjectDraft(shop());
		const changed = shop();
		changed.name = 'Changed elsewhere';

		expect(draft.isStale(shop())).toBe(false);
		expect(draft.isStale(changed)).toBe(true);
	});

	it('rebase() adopts the backend project as baseline but keeps the edits', () => {
		const draft = new ProjectDraft(shop());
		draft.name = 'Mine';
		const changed = shop();
		changed.name = 'Theirs';

		draft.rebase(changed);

		expect(draft.name).toBe('Mine');
		expect(draft.dirty).toBe(true);
		expect(draft.isStale(changed)).toBe(false);
	});

	it('reset() discards the edits and takes the project as the new baseline', () => {
		const draft = new ProjectDraft(shop());
		draft.name = 'Mine';
		const changed = shop();
		changed.name = 'Theirs';

		draft.reset(changed);

		expect(draft.name).toBe('Theirs');
		expect(draft.dirty).toBe(false);
	});
});

describe('ProjectDraft validity and request', () => {
	it('is valid as loaded', () => {
		expect(new ProjectDraft(shop()).valid).toBe(true);
	});

	it('is invalid with a blank name, no address or an empty address', () => {
		const draft = new ProjectDraft(shop());

		draft.name = '   ';
		expect(draft.valid).toBe(false);
		draft.name = 'Shop';
		expect(draft.valid).toBe(true);

		draft.addresses.setHost(draft.addresses.rows[0].id, '  ');
		expect(draft.valid).toBe(false);

		for (const row of [...draft.addresses.rows]) draft.addresses.remove(row.id);
		expect(draft.valid).toBe(false);
	});

	it('is invalid while an env var is wrong', () => {
		const draft = new ProjectDraft(shop());

		draft.envVars.setName(draft.envVars.rows[0].id, 'not a name');

		expect(draft.valid).toBe(false);
	});

	it('toRequest() trims the name and sends the database name while MySQL is used', () => {
		const draft = new ProjectDraft(shop());
		draft.name = '  Shop 2  ';
		draft.databaseName = '  shop_two ';

		const request = draft.toRequest();

		expect(request.projectId).toBe('shop');
		expect(request.name).toBe('Shop 2');
		expect(request.databaseName).toBe('shop_two');
		expect(request.environment.envVars).toEqual(draft.envVars.values);
	});

	it('toRequest() sends the MySQL credentials, so the GUI switches a project to its own account', () => {
		const legacy = shop();
		legacy.mysqlCredentials = 'root';
		const draft = new ProjectDraft(legacy);
		expect(draft.toRequest().mysqlCredentials).toBe('root');

		draft.mysqlCredentials = 'project';
		expect(draft.toRequest().mysqlCredentials).toBe('project');
		expect(draft.isStale(legacy)).toBe(false);
	});

	it('toRequest() leaves the database name out when MySQL is not used', () => {
		const draft = new ProjectDraft(withoutMysql());
		draft.databaseName = 'leftover';

		expect(draft.toRequest().databaseName).toBeNull();
	});

	it('toRequest() sends a blank database name as null', () => {
		const draft = new ProjectDraft(shop());
		draft.databaseName = '   ';

		expect(draft.toRequest().databaseName).toBeNull();
	});

	it('toRequest() returns plain data, detached from the draft', () => {
		const draft = new ProjectDraft(shop());

		const request = draft.toRequest();
		request.addresses.length = 0;

		expect(draft.addresses.rows.length).toBeGreaterThan(0);
	});
});

describe('ProjectDraft saving', () => {
	beforeEach(() => vi.useFakeTimers());
	afterEach(() => vi.useRealTimers());

	it('saves through the store and is clean once reset to the saved project', async () => {
		const { backend } = testBackend();
		const store = new ProjectsStore(backend, testRunner());
		const draft = new ProjectDraft(shop());
		draft.name = 'Shop renamed';
		expect(draft.dirty).toBe(true);

		const saving = store.update(draft.toRequest());
		await vi.advanceTimersByTimeAsync(1000);
		const result = await saving;

		expect(result.ok).toBe(true);
		if (!result.ok) return;
		expect(result.value.name).toBe('Shop renamed');
		draft.reset(result.value);
		expect(draft.dirty).toBe(false);
	});

	it('stays dirty when the backend refuses the save', async () => {
		const { backend } = testBackend();
		const store = new ProjectsStore(backend, testRunner());
		const draft = new ProjectDraft(shop());
		draft.environment.apacheDirectives = '!!';

		const saving = store.update(draft.toRequest());
		await vi.advanceTimersByTimeAsync(1000);
		const result = await saving;

		expect(result.ok).toBe(false);
		expect(draft.dirty).toBe(true);
	});
});
