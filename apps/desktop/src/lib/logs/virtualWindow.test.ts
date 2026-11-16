import assert from 'node:assert/strict';
import { describe, test } from 'node:test';

import { isAtBottom, virtualWindow } from './virtualWindow.ts';

const base = { count: 1000, rowHeight: 20, scrollTop: 0, viewportHeight: 200, overscan: 5 };

describe('virtualWindow', () => {
	test('at the top renders the viewport plus the overscan below', () => {
		assert.deepEqual(virtualWindow(base), { start: 0, end: 16, padTop: 0, padBottom: 984 * 20 });
	});

	test('in the middle renders overscan on both sides', () => {
		const w = virtualWindow({ ...base, scrollTop: 400 });
		assert.equal(w.start, 15);
		assert.equal(w.end, 36);
		assert.equal(w.padTop + (w.end - w.start) * 20 + w.padBottom, 1000 * 20);
	});

	test('at the bottom stops at the last row', () => {
		const w = virtualWindow({ ...base, scrollTop: 1000 * 20 - 200 });
		assert.equal(w.end, 1000);
		assert.equal(w.padBottom, 0);
	});

	test('scroll offsets past the end are clamped', () => {
		const w = virtualWindow({ ...base, scrollTop: 1e9 });
		assert.deepEqual(w, { start: 1000, end: 1000, padTop: 20000, padBottom: 0 });
	});

	test('empty lists and degenerate metrics render nothing', () => {
		assert.deepEqual(virtualWindow({ ...base, count: 0 }), { start: 0, end: 0, padTop: 0, padBottom: 0 });
		assert.deepEqual(virtualWindow({ ...base, rowHeight: 0 }), { start: 0, end: 0, padTop: 0, padBottom: 0 });
		const w = virtualWindow({ ...base, scrollTop: -50, viewportHeight: -1 });
		assert.equal(w.start, 0);
		assert.equal(w.end, 6);
	});
});

describe('isAtBottom', () => {
	test('true at the exact bottom and within the tolerance', () => {
		assert.equal(isAtBottom(800, 1000, 200, 4), true);
		assert.equal(isAtBottom(797, 1000, 200, 4), true);
	});

	test('false when scrolled further up', () => {
		assert.equal(isAtBottom(700, 1000, 200, 4), false);
	});

	test('true when the content fits the viewport', () => {
		assert.equal(isAtBottom(0, 100, 200, 4), true);
	});
});
