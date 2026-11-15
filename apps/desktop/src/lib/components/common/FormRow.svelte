<script lang="ts">
	import type { Snippet } from 'svelte';

	import { Label } from '#lib/components/ui/label/index.js';

	interface Props {
		label: string;
		for?: string;
		help?: string;
		children: Snippet;
	}

	let { label, for: forId, help, children }: Props = $props();
</script>

<div class="grid grid-cols-[11rem_1fr] items-start gap-x-4 gap-y-1 py-2">
	<Label for={forId} class="pt-1.5 text-sm font-normal text-muted-foreground">{label}</Label>
	<div class="min-w-0">
		{@render children()}
		{#if help}<p class="mt-1 text-xs text-muted-foreground">{help}</p>{/if}
	</div>
</div>
