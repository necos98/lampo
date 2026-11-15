// See https://svelte.dev/docs/kit/types#app.d.ts
declare global {
	namespace App {}
	interface ImportMetaEnv {
		/** `1` = simulated backend, `fresh` = simulated backend in first-run state. */
		readonly VITE_LAMPO_MOCK?: string;
	}
}

export {};
