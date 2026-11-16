import assert from 'node:assert/strict';
import { test } from 'node:test';

import { createMatcher, matchPositions, stepMatch } from './search.ts';

test('a blank query has no matcher', () => {
	assert.equal(createMatcher(''), null);
	assert.equal(createMatcher('   '), null);
});

test('matching ignores case and treats the query literally', () => {
	const matcher = createMatcher(' Fatal ')!;
	assert.equal(matcher.test('PHP FATAL error'), true);
	assert.equal(matcher.test('fine'), false);

	const special = createMatcher('a.b (x)')!;
	assert.equal(special.test('see a.b (x) here'), true);
	assert.equal(special.test('aXb (x)'), false, 'the dot is not a wildcard');
});

test('segments mark every match and keep the rest of the text', () => {
	const matcher = createMatcher('err')!;
	assert.deepEqual(matcher.segments('Error: err at end err'), [
		{ text: 'Err', match: true },
		{ text: 'or: ', match: false },
		{ text: 'err', match: true },
		{ text: ' at end ', match: false },
		{ text: 'err', match: true }
	]);
	assert.deepEqual(matcher.segments('nothing here'), [{ text: 'nothing here', match: false }]);
	assert.deepEqual(matcher.segments(''), [{ text: '', match: false }]);
});

test('segments keep offsets right around characters outside ASCII', () => {
	const matcher = createMatcher('città')!;
	assert.deepEqual(matcher.segments('İ la CITTÀ è qui'), [
		{ text: 'İ la ', match: false },
		{ text: 'CITTÀ', match: true },
		{ text: ' è qui', match: false }
	]);
});

test('match positions follow the order of the lines', () => {
	const lines = [{ text: 'a warn' }, { text: 'ok' }, { text: 'WARN b' }];
	assert.deepEqual(matchPositions(lines, createMatcher('warn')), [0, 2]);
	assert.deepEqual(matchPositions(lines, null), []);
	assert.deepEqual(matchPositions([], createMatcher('warn')), []);
});

test('stepping wraps around and starts from either end', () => {
	assert.equal(stepMatch(-1, 3, 1), 0);
	assert.equal(stepMatch(-1, 3, -1), 2);
	assert.equal(stepMatch(0, 3, 1), 1);
	assert.equal(stepMatch(2, 3, 1), 0);
	assert.equal(stepMatch(0, 3, -1), 2);
	assert.equal(stepMatch(5, 3, 1), 0, 'a stale position starts over');
	assert.equal(stepMatch(0, 0, 1), -1);
});
