import assert from 'node:assert/strict';
import { test } from 'node:test';

import { serviceLogSource } from './serviceSource.ts';

const src = (id: string, exists = true) => ({ id, exists });
const SOURCES = [src('apache'), src('apache-error'), src('mysql-5.7'), src('mysql-5.7-error', false), src('redis'), src('error:shop')];

test('a service opens on its error log, else on its captured output', () => {
	assert.equal(serviceLogSource(SOURCES, 'apache'), 'apache-error');
	assert.equal(serviceLogSource(SOURCES, 'mysql-5.7'), 'mysql-5.7', 'the error log was never written');
	assert.equal(serviceLogSource(SOURCES, 'redis'), 'redis', 'Redis has no separate error log');
});

test('no service, an unknown one or one without files names no log', () => {
	assert.equal(serviceLogSource(SOURCES, null), null);
	assert.equal(serviceLogSource(SOURCES, 'mysql-9.9'), null);
	assert.equal(serviceLogSource([src('redis', false)], 'redis'), null);
	assert.equal(serviceLogSource([], 'apache'), null);
});
