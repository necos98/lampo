<script lang="ts">
	import ShieldAlert from '@lucide/svelte/icons/shield-alert';

	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { elevation } from '#lib/stores/index.js';
</script>

{#if elevation.pending}
	<div class="flex items-center gap-3 border-b border-amber-500/40 bg-amber-500/10 px-6 py-2.5 text-sm" role="status">
		<ShieldAlert class="size-4 shrink-0 text-amber-600 dark:text-amber-400" />
		<div class="min-w-0 flex-1">
			<p class="font-medium">{m.elevation.title}</p>
			<p class="text-xs text-muted-foreground">{m.elevation.body}</p>
		</div>
		<Button size="sm" onclick={() => elevation.apply()} disabled={elevation.applying}>
			{elevation.applying ? m.elevation.applying : m.elevation.apply}
		</Button>
	</div>
{/if}
