<script lang="ts">
	import { tick } from 'svelte';

	import { messages as m } from '#lib/messages/index.js';
	import type { LogSession } from '#lib/stores/logSession.svelte.js';

	import ErrorNotice from '../common/ErrorNotice.svelte';

	interface Props {
		session: LogSession;
		follow: boolean;
		wrap: boolean;
	}

	let { session, follow, wrap }: Props = $props();
	let scroller = $state<HTMLElement>();

	const visible = $derived(session.visible);

	// Stick to the newest line while following and not paused.
	$effect(() => {
		void visible.length;
		if (follow && !session.paused && scroller) {
			const el = scroller;
			void tick().then(() => (el.scrollTop = el.scrollHeight));
		}
	});

	const level = (text: string) => (/\b(error|fatal|critical|emerg|alert)\b|\s5\d\d\s/i.test(text) ? 'text-destructive' : /\b(warn(ing)?|deprecated|notice)\b/i.test(text) ? 'text-amber-600 dark:text-amber-400' : '');
</script>

<div class="min-h-0 flex-1 overflow-auto rounded-md border bg-card" bind:this={scroller} role="log" aria-live="off" aria-label={m.logs.title}>
	{#if session.status === 'error'}
		<div class="p-3"><ErrorNotice error={session.error} onretry={() => session.open(session.source)} /></div>
	{:else if session.status === 'loading'}
		<p class="p-3 text-sm text-muted-foreground">{m.app.loading}</p>
	{:else if !session.source}
		<p class="p-3 text-sm text-muted-foreground">{m.logs.noSource}</p>
	{:else if visible.length === 0}
		<p class="p-3 text-sm text-muted-foreground">{session.filter ? m.logs.emptyFiltered : m.logs.empty}</p>
	{:else}
		<ol class="font-mono text-xs leading-5">
			{#each visible as line (line.id)}
				<li class="px-3 hover:bg-muted/60 {wrap ? 'break-all whitespace-pre-wrap' : 'whitespace-pre'} {level(line.text)}">{line.text}</li>
			{/each}
		</ol>
	{/if}
</div>
