import { flushSync } from 'svelte';
import { onTestFinished } from 'vitest';

/**
 * Reads `read` inside an effect, the way a component's `$derived` or `$effect` does, and keeps its
 * latest value in `current()`. Call `flushSync()` after a change to let the effect re-run. The
 * effect is torn down when the test finishes.
 */
export function track<T>(read: () => T): { current: () => T } {
	let value = $state<T>() as T;
	const stop = $effect.root(() => {
		$effect(() => {
			value = read();
		});
	});
	onTestFinished(stop);
	flushSync();
	return { current: () => value };
}
