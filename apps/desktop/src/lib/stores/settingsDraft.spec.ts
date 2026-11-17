import { flushSync } from 'svelte';
import { describe, expect, it } from 'vitest';

import type { Settings } from '../api/index.js';
import { track } from '../testing/reactivity.svelte.js';
import { SettingsDraft } from './settingsDraft.svelte.js';

function settings(): Settings {
	return {
		defaultPhpVersion: '8.3',
		tld: 'test',
		apachePort: 80,
		services: [],
		loopbackPortRange: { start: 8100, end: 8199 },
		closeToTray: true,
		autoRestartServices: true,
		mysqlProjectCredentials: true,
		firstRunCompleted: true,
		defaultAddressing: 'no-elevation'
	};
}

describe('SettingsDraft baseline', () => {
	it('isStale() is false for the settings it was loaded from and true once the backend changed them', () => {
		const draft = new SettingsDraft(settings());

		expect(draft.isStale(settings())).toBe(false);
		expect(draft.isStale({ ...settings(), tld: 'local' })).toBe(true);
	});

	it('rebase() adopts the backend settings as baseline but keeps the edits', () => {
		const draft = new SettingsDraft(settings());
		draft.value.tld = 'mine';
		const theirs = { ...settings(), tld: 'theirs' };

		draft.rebase(theirs);

		expect(draft.value.tld).toBe('mine');
		expect(draft.dirty).toBe(true);
		expect(draft.isStale(theirs)).toBe(false);
	});

	it('rebase() closes the external-change notice at once', () => {
		const draft = new SettingsDraft(settings());
		draft.value.tld = 'mine';
		const theirs = { ...settings(), tld: 'theirs' };
		const changedElsewhere = track(() => draft.dirty && draft.isStale(theirs));
		expect(changedElsewhere.current()).toBe(true);

		draft.rebase(theirs);
		flushSync();

		expect(changedElsewhere.current()).toBe(false);
	});

	it('rebase() turns the draft clean when the edits match the new baseline', () => {
		const draft = new SettingsDraft(settings());
		draft.value.apachePort = 8080;
		const theirs = { ...settings(), apachePort: 8080 };
		const dirty = track(() => draft.dirty);
		expect(dirty.current()).toBe(true);

		draft.rebase(theirs);
		flushSync();

		expect(dirty.current()).toBe(false);
	});
});
