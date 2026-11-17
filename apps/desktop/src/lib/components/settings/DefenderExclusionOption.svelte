<script lang="ts">
	import Copy from '@lucide/svelte/icons/copy';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import type { Snippet } from 'svelte';

	import type { ScanExclusionChange } from '#lib/api/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { scanExclusion } from '#lib/stores/index.js';
	import { needsItCommands, scanExclusionMessage, type DefenderTarget } from '#lib/stores/scanExclusion.svelte.js';

	import ConfirmDialog from '../common/ConfirmDialog.svelte';

	interface Props {
		target: DefenderTarget;
		/** What the exclusion covers now: the executables or the folder. */
		paths: readonly string[];
		confirmBody: string;
		/** Shown instead of the paths when there is nothing to exclude (the buttons are disabled). */
		empty?: string;
		recommended?: boolean;
		/** The trade-off is a serious one: shown as a warning, not as a note. */
		warning?: boolean;
		/** The folder a `projects` request names. */
		projectsFolder?: string;
		/** Replaces the list of `paths` (e.g. to choose the folder). */
		children?: Snippet;
	}

	let { target, paths, confirmBody, empty, recommended = false, warning = false, projectsFolder, children }: Props = $props();

	const t = m.settings.defender;
	const option = $derived(t[target]);
	const pending = $derived(scanExclusion.busy);
	const disabled = $derived(pending !== null || paths.length === 0);
	const last = $derived(scanExclusion.last[target]);
	/** Defender refused the last addition or does not show it: the commands for whoever manages the PC come first. */
	const managed = $derived(needsItCommands(last));
	const copyCommands = () => void scanExclusion.copyItCommands(target, projectsFolder);
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
	{#if warning}
		<p class="mt-1 flex gap-1.5 text-xs text-amber-600 dark:text-amber-400" role="note"><TriangleAlert class="mt-px size-3.5 shrink-0" aria-hidden="true" />{option.risk}</p>
	{:else}
		<p class="mt-1 text-xs text-muted-foreground">{option.risk}</p>
	{/if}

	{#if children}
		{@render children()}
	{:else if paths.length === 0}
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
		{#if !managed}
			<Button size="sm" variant="ghost" class="mr-auto" title={t.itCommands.help} disabled={paths.length === 0} onclick={copyCommands}><Copy />{t.itCommands.copy}</Button>
		{/if}
		<Button size="sm" variant="outline" {disabled} onclick={() => scanExclusion.remove(target, projectsFolder)}>{label('remove')}</Button>
		<Button size="sm" variant="outline" {disabled} onclick={() => (confirmAdd = true)}>{label('add')}</Button>
	</div>
	{#if last}
		<p class="mt-2 text-xs" role="status">
			<span class="text-muted-foreground">{t.last}:</span>
			{scanExclusionMessage(target, last)}
		</p>
		{#if managed}
			<div class="mt-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3" role="note">
				<p class="text-xs">{t.itCommands.note}</p>
				<div class="mt-2 flex justify-end">
					<Button size="sm" title={t.itCommands.help} onclick={copyCommands}><Copy />{t.itCommands.copy}</Button>
				</div>
			</div>
		{/if}
	{/if}

	<ConfirmDialog
		bind:open={confirmAdd}
		title={option.confirmAdd.title}
		body={confirmBody}
		confirmLabel={option.confirmAdd.confirm}
		destructive={warning}
		onconfirm={() => scanExclusion.add(target, projectsFolder)}
	/>
</section>
