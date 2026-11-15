<script lang="ts">
	import type { Snippet } from 'svelte';

	import { openExternal } from '#lib/services/platform.js';

	interface Props {
		href: string;
		class?: string;
		title?: string;
		children: Snippet;
	}

	let { href, class: className = '', title, children }: Props = $props();
</script>

<!-- Opens in the default browser; never navigates the app window. -->
<a
	{href}
	{title}
	target="_blank"
	rel="noopener noreferrer"
	class="font-mono text-xs whitespace-nowrap text-primary underline-offset-4 hover:underline focus-visible:underline focus-visible:outline-none {className}"
	onclick={(e) => {
		e.preventDefault();
		e.stopPropagation();
		openExternal(href);
	}}
>
	{@render children()}
</a>
