import { flushSync } from 'svelte';
import { describe, expect, it } from 'vitest';

import { track } from '../testing/reactivity.svelte.js';
import { Baseline } from './baseline.svelte.js';

describe('Baseline', () => {
	it('matches the value it was set to, and equal copies of it', () => {
		const baseline = new Baseline();
		baseline.set({ tld: 'test', ports: [80, 443] });

		expect(baseline.matches({ tld: 'test', ports: [80, 443] })).toBe(true);
		expect(baseline.matches({ tld: 'local', ports: [80, 443] })).toBe(false);
		expect(baseline.matches({ tld: 'test', ports: [80] })).toBe(false);
	});

	it('a later set replaces the snapshot (rebase keeps edits, drops the old baseline)', () => {
		const baseline = new Baseline();
		baseline.set(['old']);
		baseline.set(['new']);

		expect(baseline.matches(['old'])).toBe(false);
		expect(baseline.matches(['new'])).toBe(true);
	});

	it('the snapshot is a copy: mutating the source afterwards does not move the baseline', () => {
		const source = { name: 'shop' };
		const baseline = new Baseline();
		baseline.set(source);
		source.name = 'renamed';

		expect(baseline.matches(source)).toBe(false);
		expect(baseline.matches({ name: 'shop' })).toBe(true);
	});

	it('null and unicode values compare by content', () => {
		const baseline = new Baseline();
		baseline.set([null, 'caffè ☕']);

		expect(baseline.matches([null, 'caffè ☕'])).toBe(true);
		expect(baseline.matches(['', 'caffè ☕'])).toBe(false);
	});

	it('a set re-runs what was derived from matches()', () => {
		const baseline = new Baseline();
		baseline.set(['old']);
		const matchesNew = track(() => baseline.matches(['new']));
		expect(matchesNew.current()).toBe(false);

		baseline.set(['new']);
		flushSync();

		expect(matchesNew.current()).toBe(true);
	});
});
