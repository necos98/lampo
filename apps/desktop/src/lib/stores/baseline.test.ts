import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Baseline } from './baseline.ts';

test('matches the value it was set to, and equal copies of it', () => {
	const baseline = new Baseline();
	baseline.set({ tld: 'test', ports: [80, 443] });

	assert.equal(baseline.matches({ tld: 'test', ports: [80, 443] }), true);
	assert.equal(baseline.matches({ tld: 'local', ports: [80, 443] }), false);
	assert.equal(baseline.matches({ tld: 'test', ports: [80] }), false);
});

test('a later set replaces the snapshot (rebase keeps edits, drops the old baseline)', () => {
	const baseline = new Baseline();
	baseline.set(['old']);
	baseline.set(['new']);

	assert.equal(baseline.matches(['old']), false);
	assert.equal(baseline.matches(['new']), true);
});

test('the snapshot is a copy: mutating the source afterwards does not move the baseline', () => {
	const source = { name: 'shop' };
	const baseline = new Baseline();
	baseline.set(source);
	source.name = 'renamed';

	assert.equal(baseline.matches(source), false);
	assert.equal(baseline.matches({ name: 'shop' }), true);
});

test('null and unicode values compare by content', () => {
	const baseline = new Baseline();
	baseline.set([null, 'caffè ☕']);

	assert.equal(baseline.matches([null, 'caffè ☕']), true);
	assert.equal(baseline.matches(['', 'caffè ☕']), false);
});
