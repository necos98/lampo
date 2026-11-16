import assert from 'node:assert/strict';
import { test } from 'node:test';

import { DEFAULT_SCAFFOLD_CHOICES, SCAFFOLD_CHOICES_KEY, loadScaffoldChoices, parseScaffoldChoices, rememberScaffoldChoices } from './scaffoldChoices.ts';

/** In-memory `Storage` stand-in. */
function memoryStorage(initial: Record<string, string> = {}) {
	const data = new Map(Object.entries(initial));
	return { data, getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v) };
}

const throwing = () => {
	throw new Error('SecurityError: access denied');
};

const refusing = {
	getItem: () => null,
	setItem: () => {
		throw new Error('QuotaExceededError');
	}
};

test('nothing stored means the defaults', () => {
	assert.deepEqual(parseScaffoldChoices(null), DEFAULT_SCAFFOLD_CHOICES);
	assert.deepEqual(loadScaffoldChoices(() => memoryStorage()), DEFAULT_SCAFFOLD_CHOICES);
});

test('the choices of the last creation are remembered and read back', () => {
	const storage = memoryStorage();
	assert.equal(rememberScaffoldChoices({ parentDir: '  C:\\sites  ', starterKit: 'vue', testing: 'phpunit' }, () => storage), true);
	assert.deepEqual(loadScaffoldChoices(() => storage), { parentDir: 'C:\\sites', starterKit: 'vue', testing: 'phpunit' });
});

test('fields left out keep their remembered value (a git clone only updates the folder)', () => {
	const storage = memoryStorage();
	rememberScaffoldChoices({ parentDir: 'C:\\sites', starterKit: 'livewire', testing: 'phpunit' }, () => storage);
	rememberScaffoldChoices({ parentDir: 'D:\\work' }, () => storage);
	assert.deepEqual(loadScaffoldChoices(() => storage), { parentDir: 'D:\\work', starterKit: 'livewire', testing: 'phpunit' });
});

test('only the known fields are stored', () => {
	const storage = memoryStorage();
	rememberScaffoldChoices({ parentDir: 'C:\\sites', extra: 'x' } as never, () => storage);
	assert.deepEqual(JSON.parse(storage.data.get(SCAFFOLD_CHOICES_KEY)!), { parentDir: 'C:\\sites', starterKit: 'none', testing: 'pest' });
});

test('unknown, malformed or partial values fall back field by field', () => {
	assert.deepEqual(parseScaffoldChoices('{"parentDir":"C:\\\\sites","starterKit":"angular","testing":7}'), { parentDir: 'C:\\sites', starterKit: 'none', testing: 'pest' });
	assert.deepEqual(parseScaffoldChoices('{"parentDir":42,"starterKit":"react"}'), { parentDir: '', starterKit: 'react', testing: 'pest' });
	for (const raw of ['not json', '[]', 'null', '"C:\\\\sites"', '{']) assert.deepEqual(parseScaffoldChoices(raw), DEFAULT_SCAFFOLD_CHOICES, raw);
});

test('a parsed value is a fresh copy, never the shared default', () => {
	const parsed = parseScaffoldChoices(null);
	assert.notEqual(parsed, DEFAULT_SCAFFOLD_CHOICES);
});

test('a storage that is missing, throws or refuses the write is ignored', () => {
	assert.deepEqual(loadScaffoldChoices(throwing), DEFAULT_SCAFFOLD_CHOICES);
	assert.deepEqual(loadScaffoldChoices(() => null), DEFAULT_SCAFFOLD_CHOICES);
	assert.deepEqual(loadScaffoldChoices(() => ({ getItem: throwing, setItem: throwing })), DEFAULT_SCAFFOLD_CHOICES);
	assert.equal(rememberScaffoldChoices({ parentDir: 'C:\\sites' }, throwing), false);
	assert.equal(rememberScaffoldChoices({ parentDir: 'C:\\sites' }, () => null), false);
	assert.equal(rememberScaffoldChoices({ parentDir: 'C:\\sites' }, () => refusing), false);
});
