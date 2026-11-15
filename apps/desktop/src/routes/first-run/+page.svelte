<script lang="ts">
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import { onMount } from 'svelte';

	import { goto } from '$app/navigation';

	import logo from '#lib/assets/favicon.svg';
	import AsyncBoundary from '#lib/components/common/AsyncBoundary.svelte';
	import ExternalLink from '#lib/components/common/ExternalLink.svelte';
	import FirstRunItemRow from '#lib/components/firstrun/FirstRunItemRow.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { describeError } from '#lib/services/errors.js';
	import { diagnostics, firstRun, tasks } from '#lib/stores/index.js';

	const t = m.firstRun;
	const items = $derived(firstRun.status.data?.items ?? []);
	const task = $derived(firstRun.task);
	const outcome = $derived(task?.outcome ?? null);
	const finished = $derived(firstRun.status.data?.completed === true);
	const vc = $derived(diagnostics.doctor.data?.missingVcRuntimes ?? []);

	onMount(() => {
		void firstRun.load();
		void diagnostics.runDoctor();
	});

	// Someone opened the wizard although everything is installed (e.g. a stale link).
	$effect(() => {
		if (finished && !task && !firstRun.running) void goto('/projects', { replace: true });
	});
</script>

<div class="mx-auto flex min-h-full max-w-xl flex-col justify-center gap-5 px-6 py-10">
	<header class="flex flex-col gap-2">
		<div class="flex items-center gap-2">
			<img src={logo} alt="" class="size-8" />
			<h1 class="text-2xl font-semibold tracking-tight">{t.title}</h1>
		</div>
		<p class="text-sm">{t.intro}</p>
		<p class="text-sm text-muted-foreground">{t.explain}</p>
	</header>

	{#if vc.length}
		<div class="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2.5 text-sm" role="alert">
			<p class="flex items-center gap-2 font-medium"><TriangleAlert class="size-4 text-amber-600" />{t.vcTitle}</p>
			<p class="mt-1 text-xs text-muted-foreground">{t.vcBody}</p>
			<ul class="mt-2 flex flex-col gap-1">
				{#each vc as issue (issue.redistributable)}
					<li class="flex items-center justify-between gap-2 text-xs">
						<span>{issue.redistributable}</span>
						<ExternalLink href={issue.downloadUrl}>{t.vcDownload}</ExternalLink>
					</li>
				{/each}
			</ul>
		</div>
	{/if}

	<section aria-labelledby="what">
		<h2 id="what" class="mb-2 text-sm font-medium">{t.whatTitle}</h2>
		<AsyncBoundary status={firstRun.status.status} error={firstRun.status.error} onretry={firstRun.load} rows={5}>
			<ul class="flex flex-col gap-1.5">
				{#each items as item (item.kind)}
					<FirstRunItemRow {item} />
				{/each}
			</ul>
		</AsyncBoundary>
	</section>

	{#if outcome?.status === 'failed'}
		<p class="text-sm text-destructive" role="alert">{t.failed} {describeError(outcome.error)}</p>
	{:else if outcome?.status === 'cancelled'}
		<p class="text-sm text-muted-foreground">{t.cancelled}</p>
	{:else if finished && outcome?.status === 'succeeded'}
		<p class="text-sm text-emerald-600 dark:text-emerald-400">{t.done}</p>
	{/if}

	<div class="flex justify-end gap-2">
		{#if firstRun.running && task}
			<Button variant="ghost" onclick={() => tasks.cancel(task.taskId)}>{t.cancel}</Button>
		{:else if finished && outcome?.status === 'succeeded'}
			<Button onclick={() => goto('/projects')}>{t.finish}</Button>
		{:else}
			<Button disabled={firstRun.starting || firstRun.status.status !== 'ready'} onclick={() => firstRun.run()}>
				{firstRun.starting ? t.starting : outcome ? t.retry : t.start}
			</Button>
		{/if}
	</div>
</div>
