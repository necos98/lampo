import assert from 'node:assert/strict';
import { test } from 'node:test';

import { filterProjects } from './projectFilter.ts';

const shop = { name: 'Shop', id: 'shop', root: 'C:\\Siti\\shop', url: 'http://shop.test' };
const blog = { name: 'Blog Aziendale', id: 'blog', root: 'D:\\Lavoro\\blog', url: 'http://127.0.0.1:8081' };
const all = [shop, blog];

test('an empty or blank query keeps every project, in order', () => {
	assert.deepEqual(filterProjects(all, ''), all);
	assert.deepEqual(filterProjects(all, '   '), all);
});

test('matches name, id, folder and URL, ignoring case and surrounding spaces', () => {
	assert.deepEqual(filterProjects(all, ' AZIENDALE '), [blog]);
	assert.deepEqual(filterProjects(all, 'shop'), [shop]);
	assert.deepEqual(filterProjects(all, 'd:\\lavoro'), [blog]);
	assert.deepEqual(filterProjects(all, ':8081'), [blog]);
});

test('no match gives an empty list', () => {
	assert.deepEqual(filterProjects(all, 'wordpress'), []);
});

test('regex metacharacters are matched literally', () => {
	assert.deepEqual(filterProjects(all, '.*'), []);
	assert.deepEqual(filterProjects(all, '\\siti\\'), [shop]);
});

test('non-ASCII text matches case-insensitively', () => {
	const caffe = { name: 'CAFFÈ', id: 'caffe', root: 'C:\\caffe', url: 'http://caffe.test' };
	assert.deepEqual(filterProjects([caffe], 'caffè'), [caffe]);
});
