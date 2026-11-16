import assert from 'node:assert/strict';
import { test } from 'node:test';

import { exportFileName } from './exportFile.ts';

test('the suggested export name is a valid Windows file name', () => {
	assert.equal(exportFileName('error:shop'), 'error-shop.log');
	assert.equal(exportFileName('mysql-8.4-error'), 'mysql-8.4-error.log');
	assert.equal(exportFileName('php:my site/\*?'), 'php-my-site-.log');
});
