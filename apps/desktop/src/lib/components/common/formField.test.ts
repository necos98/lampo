import assert from 'node:assert/strict';
import { test } from 'node:test';

import { formFieldIds } from './formField.ts';

test('without an explicit id the control gets one generated from the row', () => {
	const { field } = formFieldIds('s1', undefined, false);
	assert.equal(field.id, 's1-control');
});

test('an explicit control id wins over the generated one', () => {
	const { field } = formFieldIds('s1', 'tld', false);
	assert.equal(field.id, 'tld');
});

test('an empty explicit id falls back to the generated one', () => {
	assert.equal(formFieldIds('s1', '', false).field.id, 's1-control');
});

test('help gets its own id and the control is described by it', () => {
	const { field, helpId } = formFieldIds('s1', 'tld', true);
	assert.equal(helpId, 's1-help');
	assert.equal(field['aria-describedby'], 's1-help');
});

test('without help the control has no description', () => {
	const { field, helpId } = formFieldIds('s1', 'tld', false);
	assert.equal(helpId, undefined);
	assert.equal(field['aria-describedby'], undefined);
});

test('rows with different uids never share ids', () => {
	const a = formFieldIds('s1', undefined, true);
	const b = formFieldIds('s2', undefined, true);
	assert.notEqual(a.field.id, b.field.id);
	assert.notEqual(a.helpId, b.helpId);
});
