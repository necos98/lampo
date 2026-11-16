import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isLevelFilter, levelOf, passesLevel } from './level.ts';

test('error keywords and 5xx statuses are errors', () => {
	assert.equal(levelOf('[php:error] PHP Fatal error: Uncaught Error'), 'error');
	assert.equal(levelOf('[core:CRITICAL] boom'), 'error');
	assert.equal(levelOf('127.0.0.1 - - "GET /checkout HTTP/1.1" 500 1021'), 'error');
});

test('warning keywords are warnings', () => {
	assert.equal(levelOf('PHP Warning: Undefined variable $user'), 'warn');
	assert.equal(levelOf('PHP Deprecated: Return type of Foo::count()'), 'warn');
	assert.equal(levelOf('[core:notice] AH00094: Command line'), 'warn');
});

test('errors win over warnings on the same line', () => {
	assert.equal(levelOf('warning: fatal error ahead'), 'error');
});

test('ordinary, empty and look-alike lines are not highlighted', () => {
	assert.equal(levelOf('127.0.0.1 - - "GET / HTTP/1.1" 200 5123'), null);
	assert.equal(levelOf(''), null);
	assert.equal(levelOf('errors_total=0 terrorism warned'), null);
	assert.equal(levelOf('port 5000 ready'), null);
	assert.equal(levelOf('città è pronta ✓'), null);
});

test('the level filter keeps lines at or above its severity', () => {
	const levels = [null, 'warn', 'error'] as const;
	assert.deepEqual(levels.map((l) => passesLevel(l, 'all')), [true, true, true]);
	assert.deepEqual(levels.map((l) => passesLevel(l, 'warn')), [false, true, true]);
	assert.deepEqual(levels.map((l) => passesLevel(l, 'error')), [false, false, true]);
});

test('only the known level filters are accepted', () => {
	assert.equal(isLevelFilter('warn'), true);
	assert.equal(isLevelFilter('debug'), false);
});
