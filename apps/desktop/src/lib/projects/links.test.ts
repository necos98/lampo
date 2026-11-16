import assert from 'node:assert/strict';
import { test } from 'node:test';

import { projectLogsHref, SITE_LOG_KINDS } from './links.ts';

test('project log links name the source of each site log', () => {
	assert.equal(projectLogsHref('shop'), '/logs?source=error:shop');
	assert.deepEqual(
		SITE_LOG_KINDS.map((kind) => projectLogsHref('my shop', kind)),
		['/logs?source=error:my%20shop', '/logs?source=access:my%20shop', '/logs?source=php:my%20shop']
	);
});
