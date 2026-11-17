import type { Environment } from 'vitest/runtime';

/**
 * Plain node, but with the modules transformed for the client as in the Tauri webview. The node
 * environment compiles `*.svelte.ts` for SSR, where `$effect` never runs and nothing is tracked,
 * so specs could not observe what the components derive from the stores.
 */
export default {
	name: 'client-node',
	viteEnvironment: 'client',
	setup: () => ({ teardown: () => {} })
} satisfies Environment;
