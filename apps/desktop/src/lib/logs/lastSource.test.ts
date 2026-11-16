import assert from 'node:assert/strict';
import { test } from 'node:test';

import { pickInitialSource, readLastSource, rememberLastSource, type SourceStorage } from './lastSource.ts';

function memoryStorage(): SourceStorage & { values: Map<string, string> } {
	const values = new Map<string, string>();
	return { values, getItem: (k) => values.get(k) ?? null, setItem: (k, v) => void values.set(k, v) };
}

const throwing: SourceStorage = {
	getItem: () => {
		throw new Error('SecurityError');
	},
	setItem: () => {
		throw new Error('QuotaExceededError');
	}
};

test('the last source is remembered and read back', () => {
	const storage = memoryStorage();
	assert.equal(readLastSource(storage), null);
	rememberLastSource('php:shop', storage);
	assert.equal(readLastSource(storage), 'php:shop');
});

test('a storage that throws or is missing is ignored', () => {
	assert.doesNotThrow(() => rememberLastSource('apache', throwing));
	assert.equal(readLastSource(throwing), null);
	assert.doesNotThrow(() => rememberLastSource('apache', null));
	assert.equal(readLastSource(null), null);
});

test('the initial source prefers the asked one, then the remembered one, then a file', () => {
	const sources = [
		{ id: 'apache', exists: false },
		{ id: 'redis', exists: true },
		{ id: 'error:shop', exists: false }
	];
	assert.equal(pickInitialSource(sources, 'error:shop', 'apache'), 'error:shop');
	assert.equal(pickInitialSource(sources, null, 'apache'), 'apache');
	assert.equal(pickInitialSource(sources, 'error:gone', 'php:gone'), 'redis', 'unlisted sources are skipped');
	assert.equal(pickInitialSource(sources, null, null), 'redis');
	assert.equal(pickInitialSource([], 'apache', 'apache'), null);
	assert.equal(pickInitialSource([{ id: 'apache', exists: false }], null, null), null);
});
