import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isSaveShortcut } from './navigation.ts';

const key = (k: string, mods: Partial<Record<'altKey' | 'ctrlKey' | 'metaKey' | 'shiftKey', boolean>> = {}) => ({
	key: k,
	altKey: false,
	ctrlKey: false,
	metaKey: false,
	shiftKey: false,
	...mods
});

test('Ctrl+S and Cmd+S save, whatever the letter case', () => {
	assert.equal(isSaveShortcut(key('s', { ctrlKey: true })), true);
	assert.equal(isSaveShortcut(key('S', { ctrlKey: true })), true);
	assert.equal(isSaveShortcut(key('s', { metaKey: true })), true);
});

test('other combinations do not save', () => {
	assert.equal(isSaveShortcut(key('s')), false, 'plain s is typing');
	assert.equal(isSaveShortcut(key('s', { ctrlKey: true, shiftKey: true })), false);
	assert.equal(isSaveShortcut(key('s', { ctrlKey: true, altKey: true })), false, 'AltGr+S on some layouts');
	assert.equal(isSaveShortcut(key('d', { ctrlKey: true })), false);
	assert.equal(isSaveShortcut(key('', { ctrlKey: true })), false);
});
