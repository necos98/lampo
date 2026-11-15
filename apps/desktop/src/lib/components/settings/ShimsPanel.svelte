<script lang="ts">
	import { onMount } from 'svelte';

	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { diagnostics } from '#lib/stores/index.js';

	import AsyncBoundary from '../common/AsyncBoundary.svelte';
	import CopyField from '../common/CopyField.svelte';

	const t = m.settings.shims;
	const shims = $derived(diagnostics.shims.data);

	onMount(() => void diagnostics.loadShims());
</script>

<h2 class="text-sm font-medium">{t.title}</h2>
<p class="mb-3 text-xs text-muted-foreground">{t.help}</p>
<AsyncBoundary status={diagnostics.shims.status} error={diagnostics.shims.error} onretry={diagnostics.loadShims} rows={2}>
	{#if shims}
		<div class="flex flex-wrap items-center gap-2">
			<Badge variant={shims.installed ? 'secondary' : 'outline'}>{shims.installed ? t.state.installed : t.state.missing}</Badge>
			<Badge variant={shims.onPath ? 'secondary' : 'outline'}>{shims.onPath ? t.state.onPath : t.state.notOnPath}</Badge>
			<Button class="ml-auto" size="sm" variant="outline" disabled={diagnostics.installingShims} onclick={() => diagnostics.installShims()}>
				{diagnostics.installingShims ? t.installing : t.install}
			</Button>
		</div>
		<div class="mt-2"><CopyField label={t.folder} value={shims.shimDir} /></div>
		{#if shims.shadowed.length}
			<p class="mt-3 text-sm font-medium">{t.shadowed}</p>
			<ul class="mt-1 flex flex-col gap-1">
				{#each shims.shadowed as s (s.tool)}
					<li class="rounded border border-amber-500/40 bg-amber-500/10 px-2.5 py-1.5 text-xs">{t.shadowedBy(s.tool, s.shadowedBy)}</li>
				{/each}
			</ul>
		{/if}
	{/if}
</AsyncBoundary>
