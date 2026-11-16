<script lang="ts">
	import ArrowDown from '@lucide/svelte/icons/arrow-down';

	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import type { LogSearch } from '#lib/stores/logSearch.svelte.js';
	import type { LogSession } from '#lib/stores/logSession.svelte.js';

	import ErrorNotice from '../common/ErrorNotice.svelte';
	import LogLines from './LogLines.svelte';

	interface Props {
		session: LogSession;
		search: LogSearch;
		wrap: boolean;
	}

	let { session, search, wrap }: Props = $props();
</script>

<div class="relative flex min-h-0 flex-1 flex-col overflow-hidden rounded-md border bg-card">
	{#if session.status === 'error'}
		<div class="p-3"><ErrorNotice error={session.error} onretry={() => session.open(session.source)} /></div>
	{:else if session.status === 'loading'}
		<p class="p-3 text-sm text-muted-foreground">{m.app.loading}</p>
	{:else if !session.source}
		<p class="p-3 text-sm text-muted-foreground">{m.logs.noSource}</p>
	{:else if session.visible.length === 0}
		<p class="p-3 text-sm text-muted-foreground">{session.filtering ? m.logs.emptyFiltered : m.logs.empty}</p>
	{:else}
		<LogLines
			lines={session.visible}
			{wrap}
			following={session.following}
			matcher={search.matcher}
			currentId={search.currentId}
			scrollRequest={search.scrollRequest}
			onscrollaway={() => session.unfollow()}
			onreachbottom={() => session.follow()}
		/>
	{/if}
	{#if !session.following && session.pendingCount > 0}
		<Button size="sm" class="absolute bottom-3 left-1/2 -translate-x-1/2 shadow-md" onclick={() => session.follow()}>
			<ArrowDown />{m.logs.newLines(session.pendingCount)}
		</Button>
	{/if}
</div>
