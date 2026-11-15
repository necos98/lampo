<script lang="ts">
	import Download from '@lucide/svelte/icons/download';
	import RefreshCw from '@lucide/svelte/icons/refresh-cw';
	import { onMount } from 'svelte';

	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Progress } from '#lib/components/ui/progress/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { updater } from '#lib/stores/index.js';

	const t = m.settings.updates;

	/** Release notes on one line, cut to `max` characters. */
	function shortNotes(notes: string, max = 220): string {
		const oneLine = notes.replace(/\s+/g, ' ').trim();
		return oneLine.length > max ? `${oneLine.slice(0, max)}…` : oneLine;
	}

	/** Local date of the release; empty when missing or invalid. */
	function formatDate(raw: string | null | undefined): string {
		if (!raw) return '';
		const d = new Date(raw);
		return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString('it-IT');
	}

	const update = $derived(updater.update);
	const phase = $derived(updater.phase);
	const date = $derived(formatDate(update?.date));
	const working = $derived(phase === 'downloading' || phase === 'installing');
	const label = $derived(phase === 'checking' ? t.checking : phase === 'downloading' ? t.downloading : phase === 'installing' ? t.installing : t.check);

	onMount(() => void updater.loadVersion());
</script>

<h2 class="text-sm font-medium">{t.title}</h2>
<p class="mb-3 text-xs text-muted-foreground">{t.help}</p>
<div class="flex flex-col gap-2">
	<div class="flex flex-wrap items-center gap-2">
		<Button size="sm" variant="outline" disabled={updater.busy} onclick={() => updater.check()}>
			<RefreshCw class="size-3.5 {updater.busy ? 'animate-spin' : ''}" />
			{label}
		</Button>
		{#if updater.current}<span class="text-xs text-muted-foreground">{t.version(updater.current)}</span>{/if}
		{#if update && phase !== 'uptodate'}<Badge>{t.newBadge(update.version)}</Badge>{/if}
	</div>

	{#if phase === 'uptodate' && updater.current}
		<p class="text-xs text-muted-foreground">{t.upToDate(updater.current)}</p>
	{/if}

	{#if update && (phase === 'available' || updater.busy)}
		<div class="flex flex-col gap-2 rounded-lg border bg-card p-3">
			<p class="text-sm font-medium">
				{t.available(update.version)}
				{#if date}<span class="font-normal text-muted-foreground"> · {date}</span>{/if}
			</p>
			{#if update.notes}<p class="text-xs text-muted-foreground">{shortNotes(update.notes)}</p>{/if}
			<p class="text-xs text-muted-foreground">{t.warning}</p>
			{#if working}
				<div class="flex items-center gap-2">
					{#if updater.percent === null}
						<div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted"><div class="h-full w-1/3 animate-pulse bg-primary"></div></div>
					{:else}
						<Progress class="flex-1" value={updater.percent} />
					{/if}
					<span class="w-9 text-right text-xs text-muted-foreground">{updater.percent === null ? '…' : `${updater.percent}%`}</span>
				</div>
			{/if}
			<div>
				<Button size="sm" disabled={updater.busy} onclick={() => updater.install()}>
					<Download class="size-3.5" />
					{t.install}
				</Button>
			</div>
		</div>
	{/if}

	{#if updater.error}<p class="text-xs text-destructive">{updater.error}</p>{/if}
</div>
