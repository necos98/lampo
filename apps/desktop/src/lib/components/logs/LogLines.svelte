<script lang="ts">
	import { tick, untrack } from 'svelte';

	import type { LogLevel } from '#lib/logs/level.js';
	import type { LogMatcher } from '#lib/logs/search.js';
	import { isAtBottom, VIRTUALIZE_FROM, virtualWindow, type VirtualWindow } from '#lib/logs/virtualWindow.js';
	import { messages as m } from '#lib/messages/index.js';
	import type { ScrollRequest } from '#lib/stores/logSearch.svelte.js';
	import type { LogEntry } from '#lib/stores/logSession.svelte.js';

	interface Props {
		lines: readonly LogEntry[];
		wrap: boolean;
		following: boolean;
		/** Highlights its matches in every rendered row. */
		matcher: LogMatcher | null;
		/** Line of the current search match, marked as such. */
		currentId: number | null;
		/** Scrolls the requested row into view; virtualization included, since rows have a fixed height. */
		scrollRequest: ScrollRequest | null;
		/** The reader scrolled up while following. */
		onscrollaway: () => void;
		/** The reader scrolled back to the last line while not following. */
		onreachbottom: () => void;
	}

	let { lines, wrap, following, matcher, currentId, scrollRequest, onscrollaway, onreachbottom }: Props = $props();

	/**
	 * Unwrapped rows are pinned to this height in pixels (not a rem class: the root font size is
	 * not 16px), because virtualization needs the exact height of every row.
	 */
	const ROW_HEIGHT_PX = 20;
	const ROW_HEIGHT = `${ROW_HEIGHT_PX}px`;
	const OVERSCAN_ROWS = 20;
	/** Zoomed windows report fractional scroll offsets; this much slack still counts as the bottom. */
	const BOTTOM_TOLERANCE_PX = 4;
	const CURRENT_MATCH_CLASS = 'bg-muted ring-1 ring-inset ring-primary/60';
	const LEVEL_CLASS: Record<Exclude<LogLevel, null>, string> = {
		error: 'text-destructive',
		warn: 'text-amber-600 dark:text-amber-400'
	};

	let scroller = $state<HTMLElement>();
	let scrollTop = $state(0);
	let viewportHeight = $state(0);
	let lastScrollTop = 0;

	// Wrapped rows have no fixed height, so they are all rendered and the browser skips the
	// off-screen ones (`content-visibility: auto` on each row) instead.
	const virtual = $derived(!wrap && lines.length > VIRTUALIZE_FROM);
	const win: VirtualWindow = $derived(
		virtual
			? virtualWindow({ count: lines.length, rowHeight: ROW_HEIGHT_PX, scrollTop, viewportHeight, overscan: OVERSCAN_ROWS })
			: { start: 0, end: lines.length, padTop: 0, padBottom: 0 }
	);
	const rows = $derived(win.start === 0 && win.end === lines.length ? lines : lines.slice(win.start, win.end));

	// Stick to the newest line while following, also after the layout changes (wrap toggled).
	$effect(() => {
		void lines;
		void wrap;
		if (following && scroller) {
			const el = scroller;
			void tick().then(() => (el.scrollTop = el.scrollHeight));
		}
	});

	// Bring the requested row into view once per request, centred. Only the request is tracked:
	// resizing or new lines must not scroll back to an old match.
	$effect(() => {
		const request = scrollRequest;
		if (!request || !scroller) return;
		const el = scroller;
		untrack(() => {
			if (virtual) el.scrollTop = Math.max(0, request.index * ROW_HEIGHT_PX - (viewportHeight - ROW_HEIGHT_PX) / 2);
			else void tick().then(() => el.querySelector(`[data-row="${request.index}"]`)?.scrollIntoView({ block: 'center' }));
		});
	});

	function onscroll(): void {
		if (!scroller) return;
		const top = scroller.scrollTop;
		const atBottom = isAtBottom(top, scroller.scrollHeight, scroller.clientHeight, BOTTOM_TOLERANCE_PX);
		// Only an upward move is the reader's: the list growing or shrinking never moves it up
		// unless the offset gets clamped, which leaves it at the bottom.
		if (following && !atBottom && top < lastScrollTop) onscrollaway();
		else if (!following && atBottom) onreachbottom();
		lastScrollTop = top;
		scrollTop = top;
	}
</script>

<div class="min-h-0 flex-1 overflow-auto [overflow-anchor:none]" bind:this={scroller} bind:clientHeight={viewportHeight} {onscroll} role="log" aria-live="off" aria-label={m.logs.title}>
	<ol class="font-mono text-xs {wrap ? '' : 'w-max min-w-full'}" style:line-height={ROW_HEIGHT} style:padding-top="{win.padTop}px" style:padding-bottom="{win.padBottom}px">
		{#each rows as line, i (line.id)}
			<li
				data-row={win.start + i}
				class="px-3 hover:bg-muted/60 {wrap ? 'break-all whitespace-pre-wrap [content-visibility:auto]' : 'whitespace-pre'} {line.level ? LEVEL_CLASS[line.level] : ''} {line.id === currentId ? CURRENT_MATCH_CLASS : ''}"
				style:height={wrap ? undefined : ROW_HEIGHT}
				style:contain-intrinsic-size={wrap ? `auto ${ROW_HEIGHT}` : undefined}
				aria-current={line.id === currentId ? 'true' : undefined}
			>{#if matcher}{#each matcher.segments(line.text) as segment, s (s)}{#if segment.match}<mark class="rounded-sm bg-amber-200 text-foreground dark:bg-amber-500/40">{segment.text}</mark>{:else}{segment.text}{/if}{/each}{:else}{line.text}{/if}</li>
		{/each}
	</ol>
</div>
