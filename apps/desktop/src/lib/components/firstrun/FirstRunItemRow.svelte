<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Circle from '@lucide/svelte/icons/circle';
	import LoaderCircle from '@lucide/svelte/icons/loader-circle';

	import type { FirstRunItem } from '#lib/api/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { firstRun } from '#lib/stores/index.js';

	import TaskProgress from '../common/TaskProgress.svelte';

	let { item }: { item: FirstRunItem } = $props();
	const t = m.firstRun;
	const info = $derived(t.items[item.kind] ?? { name: item.kind, hint: '' });
	const phase = $derived(firstRun.phaseOf(item));
	const progress = $derived(firstRun.progressOf(item));
</script>

<li class="flex items-center gap-3 rounded-md border bg-card px-3 py-2.5">
	<span class="shrink-0" aria-label={t.state[phase]}>
		{#if phase === 'done'}<CircleCheck class="size-5 text-emerald-500" />
		{:else if phase === 'active'}<LoaderCircle class="size-5 animate-spin text-primary" />
		{:else if phase === 'failed'}<CircleAlert class="size-5 text-destructive" />
		{:else}<Circle class="size-5 text-muted-foreground/50" />{/if}
	</span>
	<div class="min-w-0 flex-1">
		<p class="font-medium">{info.name}{item.series ? ` ${item.series}` : ''}</p>
		{#if phase === 'active' && progress}
			<TaskProgress {progress} />
		{:else}
			<p class="truncate text-xs text-muted-foreground">{info.hint}</p>
		{/if}
	</div>
	<span class="shrink-0 text-xs text-muted-foreground">{t.state[phase]}</span>
</li>
