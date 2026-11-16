import assert from 'node:assert/strict';
import { test } from 'node:test';

import { DEFAULT_SETTINGS_SECTION, isSavedSection, SETTINGS_SECTION_KINDS, SETTINGS_SECTIONS, sectionsOfKind, settingsSection } from './sections.ts';

test('section ids are unique', () => {
	const ids = SETTINGS_SECTIONS.map((s) => s.id);
	assert.equal(new Set(ids).size, ids.length);
});

test('the two kinds split every section, keeping the display order', () => {
	const saved = sectionsOfKind('saved').map((s) => s.id);
	const immediate = sectionsOfKind('immediate').map((s) => s.id);

	assert.deepEqual(saved, ['general', 'ports']);
	assert.deepEqual([...saved, ...immediate], SETTINGS_SECTIONS.map((s) => s.id));
});

test('only the sections of the draft wait for Save', () => {
	assert.equal(isSavedSection('general'), true);
	assert.equal(isSavedSection('ports'), true);
	assert.equal(isSavedSection('updates'), false);
	assert.equal(isSavedSection('about'), false);
	assert.equal(isSavedSection('unknown'), false);
	assert.equal(isSavedSection(''), false);
});

test('the page opens on a section of the draft', () => {
	assert.equal(isSavedSection(DEFAULT_SETTINGS_SECTION), true);
});

test('every kind has at least one section, and the kinds cover them all', () => {
	for (const kind of SETTINGS_SECTION_KINDS) assert.ok(sectionsOfKind(kind).length > 0, kind);
	assert.equal(SETTINGS_SECTION_KINDS.flatMap(sectionsOfKind).length, SETTINGS_SECTIONS.length);
});

test('settingsSection finds a section by id', () => {
	assert.deepEqual(settingsSection('doctor'), { id: 'doctor', kind: 'immediate' });
	assert.deepEqual(settingsSection('ports'), { id: 'ports', kind: 'saved' });
});
