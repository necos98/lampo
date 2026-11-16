import assert from 'node:assert/strict';
import { test } from 'node:test';

import { checkEnvVars } from './envVars.ts';

test('the mock takes what the backend takes', () => {
	assert.equal(checkEnvVars({}), null);
	assert.equal(checkEnvVars({ APP_ENV: 'local', _X: 'a\tb' }), null);
});

test('the mock refuses names Lampo manages, in any case', () => {
	assert.deepEqual(checkEnvVars({ Path: 'x' })?.details, { field: 'envVars', name: 'Path', part: 'name', reserved: true, reason: 'it is managed by Lampo' });
	assert.equal(checkEnvVars({ LAMPO_DB_HOST: 'x' })?.details.reserved, true);
});

test('the mock refuses a value as the backend does, without repeating it', () => {
	const refusal = checkEnvVars({ API_KEY: 'secret${HOME}' });
	assert.equal(refusal?.details.part, 'value');
	assert.equal(refusal?.details.name, 'API_KEY');
	assert.ok(!JSON.stringify(refusal).includes('secret'));
});

test('the mock refuses bad names and too many variables', () => {
	assert.equal(checkEnvVars({ '1A': 'x' })?.details.reserved, false);
	const many = Object.fromEntries(Array.from({ length: 257 }, (_, i) => [`V${i}`, '']));
	assert.equal(checkEnvVars(many)?.details.name, undefined);
});
