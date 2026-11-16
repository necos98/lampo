import assert from 'node:assert/strict';
import { describe, test } from 'node:test';

import { appendCapped, TailBuffer } from './tailBuffer.ts';

describe('appendCapped', () => {
	test('appends while under the cap', () => {
		assert.deepEqual(appendCapped([1, 2], [3], 5), [1, 2, 3]);
	});

	test('drops the oldest items past the cap', () => {
		assert.deepEqual(appendCapped([1, 2, 3], [4, 5], 4), [2, 3, 4, 5]);
	});

	test('keeps only the newest extras when they alone exceed the cap', () => {
		assert.deepEqual(appendCapped([1], [2, 3, 4, 5], 3), [3, 4, 5]);
	});

	test('returns a new array and leaves the inputs alone', () => {
		const current = [1, 2];
		const result = appendCapped(current, [], 5);
		assert.notEqual(result, current);
		assert.deepEqual(current, [1, 2]);
	});
});

describe('TailBuffer', () => {
	test('following shows new lines at once', () => {
		const buffer = new TailBuffer<number>(10);
		buffer.reset([1, 2]);
		buffer.push([3]);
		assert.deepEqual(buffer.lines, [1, 2, 3]);
		assert.equal(buffer.pendingCount, 0);
	});

	test('not following holds new lines back and counts them', () => {
		const buffer = new TailBuffer<number>(10);
		buffer.reset([1]);
		buffer.unfollow();
		const before = buffer.lines;
		buffer.push([2, 3]);
		buffer.push([4]);
		assert.equal(buffer.lines, before);
		assert.equal(buffer.pendingCount, 3);
		buffer.follow();
		assert.deepEqual(buffer.lines, [1, 2, 3, 4]);
		assert.equal(buffer.pendingCount, 0);
		assert.equal(buffer.following, true);
	});

	test('pending lines are capped but still counted', () => {
		const buffer = new TailBuffer<number>(3);
		buffer.unfollow();
		buffer.push([1, 2, 3, 4, 5]);
		assert.equal(buffer.pendingCount, 5);
		buffer.follow();
		assert.deepEqual(buffer.lines, [3, 4, 5]);
	});

	test('empty pushes do not replace the lines', () => {
		const buffer = new TailBuffer<number>(3);
		buffer.reset([1]);
		const before = buffer.lines;
		buffer.push([]);
		assert.equal(buffer.lines, before);
	});

	test('reset and clear drop pending lines but keep the follow mode', () => {
		const buffer = new TailBuffer<number>(5);
		buffer.unfollow();
		buffer.push([1]);
		buffer.clear();
		assert.deepEqual(buffer.lines, []);
		assert.equal(buffer.pendingCount, 0);
		assert.equal(buffer.following, false);
		buffer.reset([7, 8, 9, 10, 11, 12]);
		assert.deepEqual(buffer.lines, [8, 9, 10, 11, 12]);
	});

	test('follow while already following changes nothing', () => {
		const buffer = new TailBuffer<number>(5);
		buffer.reset([1]);
		const before = buffer.lines;
		buffer.follow();
		assert.equal(buffer.lines, before);
	});
});
