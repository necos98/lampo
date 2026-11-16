<script lang="ts">
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import ChevronUp from '@lucide/svelte/icons/chevron-up';

	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { LOG_SEARCH_INPUT_ID } from '#lib/logs/search.js';
	import { messages as m } from '#lib/messages/index.js';
	import type { LogSearch } from '#lib/stores/logSearch.svelte.js';

	let { search, disabled = false }: { search: LogSearch; disabled?: boolean } = $props();

	const t = m.logs;
	const hasMatches = $derived(search.matches.length > 0);
	const status = $derived(search.matcher ? (hasMatches ? t.matchPosition(search.position + 1, search.matches.length) : t.noMatches) : '');

	function onkeydown(e: KeyboardEvent): void {
		if (e.key !== 'Enter') return;
		e.preventDefault();
		if (e.shiftKey) search.previous();
		else search.next();
	}
</script>

<div class="flex items-center gap-1">
	<Input
		id={LOG_SEARCH_INPUT_ID}
		type="search"
		class="h-8 w-48"
		placeholder={t.search}
		aria-label={t.searchLabel}
		title={t.searchHelp}
		bind:value={search.query}
		{disabled}
		{onkeydown}
	/>
	<Button variant="ghost" size="icon-sm" aria-label={t.previousMatch} title={t.previousMatch} disabled={!hasMatches} onclick={() => search.previous()}><ChevronUp /></Button>
	<Button variant="ghost" size="icon-sm" aria-label={t.nextMatch} title={t.nextMatch} disabled={!hasMatches} onclick={() => search.next()}><ChevronDown /></Button>
	<span class="min-w-16 text-xs text-muted-foreground" aria-live="polite">{status}</span>
</div>
