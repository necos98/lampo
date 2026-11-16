<script lang="ts">
	import { tick } from 'svelte';

	import type { LogLevel } from '#lib/logs/level.js';
	import { isAtBottom, VIRTUALIZE_FROM, virtualWindow, type VirtualWindow } from '#lib/logs/virtualWindow.js';
	import { messages as m } from '#lib/messages/index.js';
	import type { LogEntry } from '#lib/stores/logSession.svelte.js';

	interface Props {
		lines: readonly LogEntry[];
		wrap: boolean;
		following: boolean;
		/** The reader scrolled up while following. */
		onscrollaway: () => void;
		/** The reader scrolled back to the last line while not following. */
		onreachbottom: () => void;
	}

	let { lines, wrap, following, onscrollaway, onreachbottom }: Props = $props();

	/**
	 * Unwrapped rows are pinned to this height in pixels (not a rem class: the root font size is
	 * not 16px), because virtualization needs the exact height of every row.
	 */
	const ROW_HEIGHT_PX = 20;
	const ROW_HEIGHT = `${ROW_HEIGHT_PX}px`;
	const OVERSCAN_ROWS = 20;
	/** Zoomed windows report fractional scroll offsets; this much slack still counts as the bottom. */
	const BOTTOM_TOLERANCE_PX = 4;
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
		{#each rows as line (line.id)}
			<li
				class="px-3 hover:bg-muted/60 {wrap ? 'break-all whitespace-pre-wrap [content-visibility:auto]' : 'whitespace-pre'} {line.level ? LEVEL_CLASS[line.level] : ''}"
				style:height={wrap ? undefined : ROW_HEIGHT}
				style:contain-intrinsic-size={wrap ? `auto ${ROW_HEIGHT}` : undefined}
			>{line.text}</li>
		{/each}
	</ol>
</div>
