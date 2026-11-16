<script lang="ts">
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import { onMount } from 'svelte';

	import type { Preset } from '#lib/api/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { presets } from '#lib/stores/index.js';

	import AsyncBoundary from '../common/AsyncBoundary.svelte';
	import ConfirmDialog from '../common/ConfirmDialog.svelte';

	const t = m.settings.presets;
	const custom = $derived((presets.list.data ?? []).filter((p) => !p.builtin));

	let target = $state<Preset | null>(null);
	let confirmOpen = $state(false);

	onMount(() => void presets.load());

	function ask(preset: Preset) {
		target = preset;
		confirmOpen = true;
	}

	// `target` stays set: the dialog keeps showing its name while it closes or reports a failure.
	const remove = () => (target ? presets.remove(target.id) : null);
</script>

<h2 class="text-sm font-medium">{t.title}</h2>
<p class="mb-3 text-xs text-muted-foreground">{t.help}</p>
<AsyncBoundary status={presets.list.status} error={presets.list.error} onretry={presets.load} rows={2}>
	{#if custom.length === 0}
		<p class="text-sm text-muted-foreground">{t.empty}</p>
	{:else}
		<ul class="flex flex-col gap-1">
			{#each custom as preset (preset.id)}
				<li class="flex items-center gap-3 rounded-md border bg-card px-3 py-2 text-sm">
					<div class="min-w-0 flex-1">
						<p class="truncate">{preset.name}</p>
						<p class="truncate text-xs text-muted-foreground">PHP {preset.environment.phpVersion}{preset.description ? ` · ${preset.description}` : ''}</p>
					</div>
					<Button variant="ghost" size="icon-xs" onclick={() => ask(preset)} aria-label={t.deleteLabel(preset.name)}><Trash2 /></Button>
				</li>
			{/each}
		</ul>
	{/if}
</AsyncBoundary>

<ConfirmDialog bind:open={confirmOpen} title={t.deleteTitle} body={t.deleteBody(target?.name ?? '')} confirmLabel={m.common.delete} destructive onconfirm={remove} />
