import { describe, expect, it } from 'vitest';

import { BulkConfirmation } from './bulkConfirmation.svelte.js';

describe('BulkConfirmation', () => {
	it('states the number of projects counted when it opened', () => {
		const live = { true: 3, false: 1 };
		const confirmation = new BulkConfirmation((active) => live[`${active}`]);

		confirmation.ask(true);
		expect(confirmation).toMatchObject({ open: true, target: true, count: 3 });

		// Once confirmed, the projects being switched leave the live count.
		live.true = 0;
		expect(confirmation.count).toBe(3);
	});

	it('counts again for each opening', () => {
		const live = { true: 3, false: 1 };
		const confirmation = new BulkConfirmation((active) => live[`${active}`]);
		confirmation.ask(true);
		confirmation.open = false;

		live.false = 2;
		confirmation.ask(false);

		expect(confirmation).toMatchObject({ open: true, target: false, count: 2 });
	});
});
