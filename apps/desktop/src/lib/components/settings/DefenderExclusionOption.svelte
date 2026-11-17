<script lang="ts">
	import type { ScanExclusionChange } from '#lib/api/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { scanExclusion } from '#lib/stores/index.js';
	import { scanExclusionMessage, type DefenderTarget } from '#lib/stores/scanExclusion.svelte.js';

	import ConfirmDialog from '../common/ConfirmDialog.svelte';

	interface Props {
		target: DefenderTarget;
		/** What the exclusion covers now: the executables or the folder. */
		paths: readonly string[];
		confirmBody: string;
		/** Shown instead of the paths when there is nothing to exclude (the buttons are disabled). */
		empty?: string;
		recommended?: boolean;
	}

	let { target, paths, confirmBody, empty, recommended = false }: Props = $props();

	const t = m.settings.defender;
	const option = $derived(t[target]);
	const pending = $derived(scanExclusion.busy);
	const disabled = $derived(pending !== null || paths.length === 0);
	const last = $derived(scanExclusion.last[target]);
	const label = (change: ScanExclusionChange) => {
		const running = pending?.target === target && pending.change === change;
		if (change === 'add') return running ? t.adding : t.add;
		return running ? t.removing : t.remove;
	};
	let confirmAdd = $state(false);
</script>

<section class="mt-4" aria-labelledby="defender-{target}">
	<div class="flex items-center gap-2">
		<h3 id="defender-{target}" class="text-sm font-medium">{option.title}</h3>
		{#if recommended}<Badge variant="secondary">{t.recommended}</Badge>{/if}
	</div>
	<p class="text-xs text-muted-foreground">{option.help}</p>
	<p class="mt-1 text-xs text-muted-foreground">{option.risk}</p>

	{#if paths.length === 0}
		<p class="mt-2 text-xs">{empty}</p>
	{:else}
		<p class="mt-2 text-xs text-muted-foreground">{option.targets}</p>
		<ul class="mt-1 space-y-1">
			{#each paths as path (path)}
				<li><code class="block truncate rounded bg-muted px-2 py-1 font-mono text-xs" title={path}>{path}</code></li>
			{/each}
		</ul>
	{/if}

	<div class="mt-2 flex flex-wrap items-center justify-end gap-2">
		<Button size="sm" variant="outline" {disabled} onclick={() => scanExclusion.remove(target)}>{label('remove')}</Button>
		<Button size="sm" variant="outline" {disabled} onclick={() => (confirmAdd = true)}>{label('add')}</Button>
	</div>
	{#if last}
		<p class="mt-2 text-xs" role="status">
			<span class="text-muted-foreground">{t.last}:</span>
			{scanExclusionMessage(target, last)}
		</p>
	{/if}

	<ConfirmDialog
		bind:open={confirmAdd}
		title={option.confirmAdd.title}
		body={confirmBody}
		confirmLabel={option.confirmAdd.confirm}
		onconfirm={() => scanExclusion.add(target)}
	/>
</section>
