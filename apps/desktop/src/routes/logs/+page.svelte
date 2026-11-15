<script lang="ts">
	import Pause from '@lucide/svelte/icons/pause';
	import Play from '@lucide/svelte/icons/play';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import { onMount } from 'svelte';

	import { page } from '$app/state';

	import AsyncBoundary from '#lib/components/common/AsyncBoundary.svelte';
	import PageHeader from '#lib/components/common/PageHeader.svelte';
	import LogSourceSelect from '#lib/components/logs/LogSourceSelect.svelte';
	import LogViewer from '#lib/components/logs/LogViewer.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Checkbox } from '#lib/components/ui/checkbox/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { createLogSession, logSources } from '#lib/stores/index.js';

	const t = m.logs;
	const session = createLogSession();
	let follow = $state(true);
	let wrap = $state(false);

	onMount(() => {
		void logSources.load().then(() => {
			const wanted = page.url.searchParams.get('source');
			const all = logSources.list.data ?? [];
			const pick = all.find((s) => s.id === wanted) ?? all.find((s) => s.exists);
			if (pick) void session.open(pick.id);
		});
		return () => session.close();
	});

	const sources = $derived(logSources.list.data ?? []);
</script>

<div class="flex h-full flex-col">
	<PageHeader title={t.title}>
		{#snippet actions()}
			<LogSourceSelect value={session.source} onchange={(id) => session.open(id)} />
		{/snippet}
	</PageHeader>

	<AsyncBoundary status={logSources.list.status} error={logSources.list.error} onretry={logSources.load} empty={sources.length === 0} emptyMessage={t.noSources} rows={2}>
		<div class="mb-2 flex flex-wrap items-center gap-3">
			<Input data-filter class="h-8 max-w-xs" placeholder={t.filter} bind:value={session.filter} aria-label={t.filter} />
			<Button variant="outline" size="sm" onclick={() => (session.paused ? session.resume() : session.pause())} disabled={!session.source}>
				{#if session.paused}<Play />{t.resume}{:else}<Pause />{t.pause}{/if}
			</Button>
			<Button variant="ghost" size="sm" onclick={() => session.clear()} disabled={!session.source}><Trash2 />{t.clear}</Button>
			<div class="flex items-center gap-1.5"><Checkbox id="follow" bind:checked={follow} /><Label for="follow" class="text-xs font-normal">{t.follow}</Label></div>
			<div class="flex items-center gap-1.5"><Checkbox id="wrap" bind:checked={wrap} /><Label for="wrap" class="text-xs font-normal">{t.wrap}</Label></div>
			{#if session.source}
				<span class="ml-auto text-xs text-muted-foreground">
					{session.paused ? t.livePaused : t.live} · {t.lines(session.visible.length, session.lines.length)}
				</span>
			{/if}
		</div>
		<LogViewer {session} {follow} {wrap} />
		{#if session.file}<p class="mt-1.5 truncate font-mono text-[11px] text-muted-foreground">{session.file}</p>{/if}
	</AsyncBoundary>
</div>
