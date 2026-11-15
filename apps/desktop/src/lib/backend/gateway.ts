import { createTauriBackend } from './tauri.js';
import type { Backend } from './ports.js';

/**
 * Backend selection. `VITE_LAMPO_MOCK=1` (or `fresh` for a first-run state) swaps in the
 * simulated backend; the mock module is a dynamic import so production bundles never contain it.
 */
const flag = import.meta.env.VITE_LAMPO_MOCK as string | undefined;

export const backend: Backend = flag
	? (await import('./mock/index.js')).createMockBackend({ fresh: flag === 'fresh' })
	: createTauriBackend();
