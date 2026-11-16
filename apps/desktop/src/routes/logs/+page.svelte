<script lang="ts">
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import { onMount } from 'svelte';

	import { page } from '$app/state';

	import AsyncBoundary from '#lib/components/common/AsyncBoundary.svelte';
	import PageHeader from '#lib/components/common/PageHeader.svelte';
	import SelectField from '#lib/components/common/SelectField.svelte';
	import LogActions from '#lib/components/logs/LogActions.svelte';
	import LogSearchBar from '#lib/components/logs/LogSearchBar.svelte';
	import LogSourceSelect from '#lib/components/logs/LogSourceSelect.svelte';
	import LogViewer from '#lib/components/logs/LogViewer.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Checkbox } from '#lib/components/ui/checkbox/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { pickInitialSource, readLastSource, rememberLastSource } from '#lib/logs/lastSource.js';
	import { isLevelFilter, LEVEL_FILTERS } from '#lib/logs/level.js';
	import { messages as m } from '#lib/messages/index.js';
	import { createLogSearch, createLogSession, logSources } from '#lib/stores/index.js';

	const t = m.logs;
	const session = createLogSession();
	const search = createLogSearch(session);
	const levelOptions = LEVEL_FILTERS.map((value) => ({ value, label: t.levels[value] }));
	let wrap = $state(false);

	/** Opens `source` and remembers it for the next visit. */
	function show(source: string): void {
		rememberLastSource(source);
		void session.open(source);
	}

	function setLevel(value: string): void {
		if (isLevelFilter(value)) session.levelFilter = value;
	}

	onMount(() => {
		session.setHidden(document.hidden);
		void logSources.load().then(() => {
			const pick = pickInitialSource(logSources.list.data ?? [], page.url.searchParams.get('source'), readLastSource());
			if (pick) show(pick);
		});
		return () => session.close();
	});

	const sources = $derived(logSources.list.data ?? []);
</script>

<svelte:document onvisibilitychange={() => session.setHidden(document.hidden)} />

<div class="flex h-full flex-col">
	<PageHeader title={t.title}>
		{#snippet actions()}
			<LogSourceSelect value={session.source} onchange={show} />
		{/snippet}
	</PageHeader>

	<AsyncBoundary status={logSources.list.status} error={logSources.list.error} onretry={logSources.load} empty={sources.length === 0} emptyMessage={t.noSources} rows={2}>
		<div class="mb-2 flex flex-wrap items-center gap-3">
			<Input data-filter class="h-8 max-w-xs" placeholder={t.filter} bind:value={session.filter} aria-label={t.filter} />
			<SelectField class="w-44" options={levelOptions} aria-label={t.level} value={session.levelFilter} onchange={setLevel} />
			<LogSearchBar {search} disabled={!session.source} />
			<div class="flex items-center gap-1.5">
				<Checkbox id="follow" bind:checked={() => session.following, (on) => (on ? session.follow() : session.unfollow())} disabled={!session.source} />
				<Label for="follow" class="text-xs font-normal">{t.follow}</Label>
			</div>
			<div class="flex items-center gap-1.5"><Checkbox id="wrap" bind:checked={wrap} /><Label for="wrap" class="text-xs font-normal">{t.wrap}</Label></div>
		</div>
		<div class="mb-2 flex flex-wrap items-center gap-1">
			<LogActions {session} />
			<Button variant="ghost" size="sm" onclick={() => session.clear()} disabled={!session.source}><Trash2 />{t.clear}</Button>
			{#if session.source}
				<span class="ml-auto text-xs text-muted-foreground">
					{t.live} · {t.lines(session.visible.length, session.lines.length)}
				</span>
			{/if}
		</div>
		<LogViewer {session} {search} {wrap} />
		{#if session.file}<p class="mt-1.5 truncate font-mono text-[11px] text-muted-foreground">{session.file}</p>{/if}
	</AsyncBoundary>
</div>
