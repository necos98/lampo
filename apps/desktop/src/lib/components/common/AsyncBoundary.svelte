<script lang="ts" generics="T">
	import type { Snippet } from 'svelte';

	import type { AppError } from '#lib/api/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import type { ResourceStatus } from '#lib/stores/resource.svelte.js';

	import EmptyState from './EmptyState.svelte';
	import ErrorNotice from './ErrorNotice.svelte';

	interface Props {
		status: ResourceStatus;
		error: AppError | null;
		onretry: () => void;
		/** `true` when the loaded data has nothing to show. */
		empty?: boolean;
		emptyMessage?: string;
		/** Number of skeleton rows while loading. */
		rows?: number;
		emptyAction?: Snippet;
		children: Snippet;
	}

	let { status, error, onretry, empty = false, emptyMessage = '', rows = 4, emptyAction, children }: Props = $props();
</script>

{#if status === 'loading' || status === 'idle'}
	<div class="flex flex-col gap-2" aria-busy="true">
		{#each Array.from({ length: rows }, (_, i) => i) as i (i)}
			<Skeleton class="h-11 w-full" />
		{/each}
	</div>
{:else if status === 'error'}
	<ErrorNotice {error} {onretry} />
{:else if empty}
	<EmptyState message={emptyMessage}>
		{#if emptyAction}{@render emptyAction()}{/if}
	</EmptyState>
{:else}
	{@render children()}
{/if}
