import assert from 'node:assert/strict';
import { test } from 'node:test';

import { ADDRESSING_MODES, FALLBACK_ADDRESSING, effectiveAddressing, needsElevation } from './addressing.ts';

test('lists the three modes once, the one without administrator rights first', () => {
	assert.deepEqual(ADDRESSING_MODES, ['no-elevation', 'standard', 'hosts-only']);
	assert.equal(ADDRESSING_MODES[0], FALLBACK_ADDRESSING);
});

test('only the modes that write the hosts file need elevation', () => {
	assert.equal(needsElevation('no-elevation'), false);
	assert.equal(needsElevation('standard'), true);
	assert.equal(needsElevation('hosts-only'), true);
});

test('the user choice wins over the configured default', () => {
	assert.equal(effectiveAddressing('standard', 'hosts-only'), 'standard');
});

test('without a choice the configured default applies', () => {
	assert.equal(effectiveAddressing(null, 'hosts-only'), 'hosts-only');
});

test('settings not loaded yet fall back to the built-in default', () => {
	assert.equal(effectiveAddressing(null, undefined), 'no-elevation');
	assert.equal(effectiveAddressing(null, null), 'no-elevation');
});
