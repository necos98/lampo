<script lang="ts">
	import type { Snippet } from 'svelte';

	let { message, children }: { message: string; children?: Snippet } = $props();
</script>

<div class="flex flex-col items-center gap-3 rounded-lg border border-dashed py-10 text-center">
	<p class="max-w-sm text-sm text-muted-foreground">{message}</p>
	{@render children?.()}
</div>
