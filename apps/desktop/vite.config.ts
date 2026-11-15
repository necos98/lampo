import tailwindcss from '@tailwindcss/vite';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

// Tauri expects a fixed dev port and must not have the screen cleared.
// See https://v2.tauri.app/start/frontend/sveltekit/
const host = process.env.TAURI_DEV_HOST;

/**
 * Simulated backend switch: `pnpm dev:mock` (mode "mock"), `pnpm dev:mock-fresh` (first-run state)
 * or `VITE_LAMPO_MOCK=1|fresh` in the environment. Production builds never set it, so the mock
 * is dead code there and is dropped from the bundle.
 */
const MOCK_BY_MODE: Record<string, string> = { mock: '1', 'mock-fresh': 'fresh' };

export default defineConfig(({ mode }) => ({
	define: {
		'import.meta.env.VITE_LAMPO_MOCK': JSON.stringify(process.env.VITE_LAMPO_MOCK ?? MOCK_BY_MODE[mode] ?? '')
	},
	clearScreen: false,
	server: {
		port: 5173,
		strictPort: true,
		host: host || false,
		hmr: host ? { protocol: 'ws', host, port: 5174 } : undefined,
		watch: { ignored: ['**/src-tauri/**'] }
	},
	plugins: [
		tailwindcss(),
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			// Static SPA: the Tauri webview loads the files from `build/`, with no server.
			adapter: adapter({ fallback: 'index.html' })
		})
	]
}));
