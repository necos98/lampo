import assert from 'node:assert/strict';
import { test } from 'node:test';

import { conflictPortParam, portConflictHref, serviceLogsHref } from './links.ts';

test('the logs link names the service, encoded, or lets the page pick', () => {
	assert.equal(serviceLogsHref('mysql-8.4'), '/logs?service=mysql-8.4');
	assert.equal(serviceLogsHref('a&b=c'), '/logs?service=a%26b%3Dc');
	assert.equal(serviceLogsHref(null), '/logs');
});

test('the conflict link round-trips through its parameter', () => {
	const href = portConflictHref(3307);
	assert.equal(href, '/services?conflict=3307');
	assert.equal(conflictPortParam(new URL(href, 'http://lampo').searchParams.get('conflict')), 3307);
});

test('a missing or malformed conflict parameter names no port', () => {
	for (const bad of [null, '', 'abc', '80a', '-80', '8.5', '0', '65536', '999999', ' 80']) {
		assert.equal(conflictPortParam(bad), null, `${bad}`);
	}
	assert.equal(conflictPortParam('65535'), 65535);
	assert.equal(conflictPortParam('1'), 1);
});
