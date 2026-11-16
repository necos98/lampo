import assert from 'node:assert/strict';
import { test } from 'node:test';

import { DEFAULT_PROJECT_VIEW, PROJECT_VIEW_KEY, filterByStatus, loadProjectView, parseProjectView, projectStatus, saveProjectView, sortProjects } from './projectView.ts';

const shop = { id: 'shop', name: 'Shop', active: true, folderExists: true };
const blog = { id: 'blog', name: 'blog', active: false, folderExists: true };
const old = { id: 'old', name: 'Archivio', active: true, folderExists: false };
const site10 = { id: 'site10', name: 'Sito 10', active: false, folderExists: true };
const site2 = { id: 'site2', name: 'Sito 2', active: false, folderExists: true };
const all = [shop, blog, old, site10, site2];

/** In-memory `Storage` stand-in. */
function memoryStorage(initial: Record<string, string> = {}) {
	const data = new Map(Object.entries(initial));
	return { data, getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v) };
}

const throwing = () => {
	throw new Error('SecurityError: access denied');
};

test('a missing folder wins over the active flag', () => {
	assert.equal(projectStatus(shop), 'active');
	assert.equal(projectStatus(blog), 'inactive');
	assert.equal(projectStatus(old), 'missing');
	assert.equal(projectStatus({ active: false, folderExists: false }), 'missing');
});

test('the status filter keeps only the projects in that status', () => {
	assert.equal(filterByStatus(all, 'all'), all);
	assert.deepEqual(filterByStatus(all, 'active'), [shop]);
	assert.deepEqual(filterByStatus(all, 'inactive'), [blog, site10, site2]);
	assert.deepEqual(filterByStatus(all, 'missing'), [old]);
	assert.deepEqual(filterByStatus([], 'active'), []);
});

test('by name: case-insensitive, numbers in natural order, input left untouched', () => {
	const before = [...all];
	assert.deepEqual(sortProjects(all, 'name'), [old, blog, shop, site2, site10]);
	assert.deepEqual(all, before);
});

test('by status: active, inactive, missing folder, by name within each', () => {
	assert.deepEqual(sortProjects(all, 'status'), [shop, blog, site2, site10, old]);
});

test('equal names fall back to the id, so the order is stable', () => {
	const a = { ...shop, id: 'a-shop' };
	const b = { ...shop, id: 'b-shop' };
	assert.deepEqual(sortProjects([b, a], 'name'), [a, b]);
});

test('a stored view is read back; unknown, missing or malformed fields fall back to the defaults', () => {
	assert.deepEqual(parseProjectView('{"sort":"status","status":"missing"}'), { sort: 'status', status: 'missing' });
	assert.deepEqual(parseProjectView('{"sort":"lastUsed","status":"missing"}'), { sort: 'name', status: 'missing' });
	assert.deepEqual(parseProjectView('{"sort":"status"}'), { sort: 'status', status: 'all' });
	assert.deepEqual(parseProjectView(null), DEFAULT_PROJECT_VIEW);
	assert.deepEqual(parseProjectView('not json'), DEFAULT_PROJECT_VIEW);
	assert.deepEqual(parseProjectView('null'), DEFAULT_PROJECT_VIEW);
	assert.deepEqual(parseProjectView('["status"]'), DEFAULT_PROJECT_VIEW);
	assert.deepEqual(parseProjectView('{"sort":1,"status":{}}'), DEFAULT_PROJECT_VIEW);
});

test('save then load round-trips through storage', () => {
	const storage = memoryStorage();
	assert.equal(saveProjectView({ sort: 'status', status: 'inactive' }, () => storage), true);
	assert.deepEqual(JSON.parse(storage.data.get(PROJECT_VIEW_KEY) ?? ''), { sort: 'status', status: 'inactive' });
	assert.deepEqual(loadProjectView(() => storage), { sort: 'status', status: 'inactive' });
});

test('no storage, or storage that throws, gives the defaults and a failed save', () => {
	assert.deepEqual(loadProjectView(() => null), DEFAULT_PROJECT_VIEW);
	assert.deepEqual(loadProjectView(throwing), DEFAULT_PROJECT_VIEW);
	assert.equal(saveProjectView({ sort: 'name', status: 'all' }, () => null), false);
	assert.equal(saveProjectView({ sort: 'name', status: 'all' }, throwing), false);
	const full = { getItem: () => null, setItem: () => throwing() };
	assert.equal(saveProjectView({ sort: 'name', status: 'all' }, () => full), false);
	assert.deepEqual(loadProjectView(() => ({ getItem: () => throwing(), setItem: () => {} })), DEFAULT_PROJECT_VIEW);
});

test('loading returns a fresh object the caller may change', () => {
	const view = loadProjectView(() => null);
	view.sort = 'status';
	assert.equal(DEFAULT_PROJECT_VIEW.sort, 'name');
});
