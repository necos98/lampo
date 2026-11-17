import assert from 'node:assert/strict';
import { test } from 'node:test';

import { openRequestedSource, type ListedSource, type SourceRequest } from './initialSource.ts';

const SOURCES: ListedSource[] = [
	{ id: 'apache', exists: true },
	{ id: 'apache-error', exists: true },
	{ id: 'access:shop', exists: false },
	{ id: 'error:shop', exists: true },
	{ id: 'php:shop', exists: false }
];

/** Runs a request over `SOURCES`; `beforeList` runs while the list is still loading. */
async function run(request: Partial<SourceRequest>, options: { remembered?: string | null; beforeList?: (controller: AbortController) => void } = {}) {
	const controller = new AbortController();
	const shown: string[] = [];
	await openRequestedSource({
		load: async () => {
			options.beforeList?.(controller);
			return SOURCES;
		},
		request: { source: null, service: null, ...request },
		remembered: () => options.remembered ?? null,
		show: (source) => shown.push(source),
		signal: controller.signal
	});
	return shown;
}

test('a project link opens the site log it names, even without a file yet', async () => {
	assert.deepEqual(await run({ source: 'error:shop' }, { remembered: 'apache' }), ['error:shop']);
	assert.deepEqual(await run({ source: 'access:shop' }, { remembered: 'apache' }), ['access:shop']);
	assert.deepEqual(await run({ source: 'php:shop' }, { remembered: 'apache' }), ['php:shop']);
});

test('a service request opens its error log, without a request the remembered source', async () => {
	assert.deepEqual(await run({ service: 'apache' }, { remembered: 'php:shop' }), ['apache-error']);
	assert.deepEqual(await run({}, { remembered: 'php:shop' }), ['php:shop']);
});

test('a request aborted before the list arrives shows nothing', async () => {
	const shown = await run({ source: 'error:shop' }, { beforeList: (controller) => controller.abort() });

	assert.deepEqual(shown, []);
});
