import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

/**
 * Store tests (`*.spec.ts`). The svelte plugin compiles the runes of `*.svelte.ts` files. The
 * `*.test.ts` files are plain `node --test` tests (see the `test` script) and are left to node.
 */
export default defineConfig({
	plugins: [svelte({ compilerOptions: { runes: true } })],
	resolve: { conditions: ['browser'] },
	test: {
		include: ['src/**/*.spec.ts'],
		environment: 'node',
		restoreMocks: true
	}
});
