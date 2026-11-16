import assert from 'node:assert/strict';
import { test } from 'node:test';

import { UnsavedChanges, type UnsavedForm } from './unsavedChanges.ts';

/** A form whose dirtiness the test drives, counting saves and discards. */
function fakeForm(dirty: boolean) {
	const form = {
		dirty,
		saves: 0,
		discards: 0,
		async save() {
			form.saves++;
			form.dirty = false;
		},
		discard() {
			form.discards++;
			form.dirty = false;
		}
	};
	return form satisfies UnsavedForm;
}

test('nothing registered: not dirty, save and discard are no-ops', async () => {
	const unsaved = new UnsavedChanges();
	assert.equal(unsaved.dirty, false);
	await unsaved.saveAll();
	unsaved.discardAll();
});

test('dirty while any registered form is dirty, read live from the form', () => {
	const unsaved = new UnsavedChanges();
	const clean = fakeForm(false);
	const edited = fakeForm(false);
	unsaved.register(clean);
	unsaved.register(edited);

	assert.equal(unsaved.dirty, false);
	edited.dirty = true;
	assert.equal(unsaved.dirty, true);
});

test('saveAll saves only the dirty forms', async () => {
	const unsaved = new UnsavedChanges();
	const clean = fakeForm(false);
	const edited = fakeForm(true);
	unsaved.register(clean);
	unsaved.register(edited);

	await unsaved.saveAll();

	assert.equal(edited.saves, 1);
	assert.equal(clean.saves, 0);
	assert.equal(unsaved.dirty, false);
});

test('discardAll discards only the dirty forms', () => {
	const unsaved = new UnsavedChanges();
	const clean = fakeForm(false);
	const edited = fakeForm(true);
	unsaved.register(clean);
	unsaved.register(edited);

	unsaved.discardAll();

	assert.equal(edited.discards, 1);
	assert.equal(clean.discards, 0);
	assert.equal(unsaved.dirty, false);
});

test('an unregistered (unmounted) form no longer counts', async () => {
	const unsaved = new UnsavedChanges();
	const edited = fakeForm(true);
	const unregister = unsaved.register(edited);

	unregister();

	assert.equal(unsaved.dirty, false);
	await unsaved.saveAll();
	assert.equal(edited.saves, 0);
});

test('a failing save is reported to the caller', async () => {
	const unsaved = new UnsavedChanges();
	unsaved.register({
		dirty: true,
		save: () => Promise.reject(new Error('disk full')),
		discard: () => {}
	});

	await assert.rejects(unsaved.saveAll(), /disk full/);
});
