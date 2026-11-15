<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		title: string;
		subtitle?: string;
		/** Makes the subtitle a link-styled button (e.g. a path that opens the folder). */
		onsubtitleclick?: () => void;
		/** Tooltip of the clickable subtitle. */
		subtitleTitle?: string;
		/** Renders the clickable subtitle as disabled (plain text). */
		subtitleDisabled?: boolean;
		actions?: Snippet;
		/** Extra content next to the title (badges, status). */
		meta?: Snippet;
	}

	let { title, subtitle, onsubtitleclick, subtitleTitle, subtitleDisabled = false, actions, meta }: Props = $props();
</script>

<header class="flex flex-wrap items-start justify-between gap-x-4 gap-y-2 pb-4">
	<div class="min-w-0">
		<div class="flex min-w-0 items-center gap-2">
			<h1 class="min-w-0 truncate text-lg font-semibold tracking-tight">{title}</h1>
			{@render meta?.()}
		</div>
		{#if subtitle}
			{#if onsubtitleclick && !subtitleDisabled}
				<button
					type="button"
					class="block cursor-pointer rounded-sm text-left text-sm text-muted-foreground underline-offset-2 hover:text-foreground hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
					title={subtitleTitle}
					onclick={onsubtitleclick}>{subtitle}</button
				>
			{:else}
				<p class="text-sm text-muted-foreground">{subtitle}</p>
			{/if}
		{/if}
	</div>
	{#if actions}<div class="flex flex-wrap items-center gap-2">{@render actions()}</div>{/if}
</header>
