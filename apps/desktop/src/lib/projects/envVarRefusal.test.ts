import assert from 'node:assert/strict';
import { test } from 'node:test';

import { envVarRefusal } from './envVarRefusal.ts';

test('a refused name points at its row', () => {
	const details = { field: 'envVars', name: 'LAMPO_X', part: 'name', reserved: true, reason: 'it is managed by Lampo' };
	assert.deepEqual(envVarRefusal({ code: 'validation', details }), { name: 'LAMPO_X', part: 'name', reserved: true, reason: 'it is managed by Lampo' });
});

test('a refused value points at its row and is not reserved', () => {
	const details = { field: 'envVars', name: 'API_KEY', part: 'value', reason: 'it contains `${`' };
	assert.deepEqual(envVarRefusal({ code: 'validation', details }), { name: 'API_KEY', part: 'value', reserved: false, reason: 'it contains `${`' });
});

test('a refusal of all the variables names no row', () => {
	const details = { field: 'envVars', reason: 'at most 256 variables per project' };
	assert.deepEqual(envVarRefusal({ code: 'validation', details }), { name: null, part: null, reserved: false, reason: 'at most 256 variables per project' });
});

test('other errors are not refusals of the variables', () => {
	assert.equal(envVarRefusal({ code: 'validation', details: { field: 'apacheDirectives', reason: 'x' } }), null);
	assert.equal(envVarRefusal({ code: 'conflict', details: { field: 'envVars', reason: 'x' } }), null);
	assert.equal(envVarRefusal({ code: 'validation', details: { field: 'envVars' } }), null);
	assert.equal(envVarRefusal({ code: 'validation' }), null);
});

test('an unknown part is ignored', () => {
	const details = { field: 'envVars', name: 'A', part: 'other', reason: 'x' };
	assert.equal(envVarRefusal({ code: 'validation', details })?.part, null);
});
