import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isSaveShortcut, matchShortcut, NAV_ITEMS, NEW_PROJECT_HREF, SHORTCUT_HELP } from './navigation.ts';

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

const outside = { typing: false, inFilter: false };
const typing = { typing: true, inFilter: false };
const inFilter = { typing: true, inFilter: true };

test('Ctrl shortcuts work everywhere, even while typing', () => {
	for (const context of [outside, typing]) {
		assert.deepEqual(matchShortcut(key('s', { ctrlKey: true }), context), { kind: 'save' });
		assert.deepEqual(matchShortcut(key('N', { ctrlKey: true }), context), { kind: 'navigate', href: NEW_PROJECT_HREF });
		assert.deepEqual(matchShortcut(key('k', { metaKey: true }), context), { kind: 'palette' });
		assert.deepEqual(matchShortcut(key('f', { ctrlKey: true }), context), { kind: 'find' });
	}
});

test('Alt+1..5 jumps to the sections in sidebar order', () => {
	NAV_ITEMS.forEach((item, i) => assert.deepEqual(matchShortcut(key(String(i + 1), { altKey: true }), typing), { kind: 'navigate', href: item.href }));
	assert.equal(matchShortcut(key(String(NAV_ITEMS.length + 1), { altKey: true }), outside), null);
	assert.equal(matchShortcut(key('0', { altKey: true }), outside), null);
	assert.equal(matchShortcut(key('a', { altKey: true }), outside), null);
});

test('single keys act outside text fields only', () => {
	assert.deepEqual(matchShortcut(key('/'), outside), { kind: 'focusFilter' });
	assert.deepEqual(matchShortcut(key('?', { shiftKey: true }), outside), { kind: 'help' });
	assert.equal(matchShortcut(key('/'), typing), null, 'a slash typed in a field');
	assert.equal(matchShortcut(key('?', { shiftKey: true }), typing), null, 'a question mark typed in a field');
	assert.equal(matchShortcut(key('a'), outside), null);
});

test('Escape leaves a list filter, and nothing else', () => {
	assert.deepEqual(matchShortcut(key('Escape'), inFilter), { kind: 'leaveFilter' });
	assert.equal(matchShortcut(key('Escape'), typing), null, 'other fields keep Escape (dialogs close on it)');
	assert.equal(matchShortcut(key('Escape'), outside), null);
});

test('AltGr (Ctrl+Alt) and Ctrl+Shift combinations are not shortcuts', () => {
	assert.equal(matchShortcut(key('k', { ctrlKey: true, altKey: true }), outside), null);
	assert.equal(matchShortcut(key('n', { ctrlKey: true, shiftKey: true }), outside), null);
	assert.equal(matchShortcut(key('1', { ctrlKey: true, altKey: true }), outside), null);
	assert.equal(matchShortcut(key('/', { ctrlKey: true }), outside), null);
});

test('the help lists every entry once', () => {
	const ids = SHORTCUT_HELP.map((entry) => entry.id);
	assert.equal(new Set(ids).size, ids.length);
	assert.ok(SHORTCUT_HELP.every((entry) => entry.keys.length > 0));
});
