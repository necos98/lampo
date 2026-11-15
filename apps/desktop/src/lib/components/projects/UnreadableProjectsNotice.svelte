<script lang="ts">
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';

	import type { UnreadableProject } from '#lib/api/index.js';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { messages as m } from '#lib/messages/index.js';

	/** Project files that do not load (never empty: the page shows this only when there are some). */
	let { unreadable }: { unreadable: UnreadableProject[] } = $props();
</script>

<Alert.Root role="status" class="mb-3">
	<TriangleAlert />
	<Alert.Title>{m.projects.unreadableTitle(unreadable.length)}</Alert.Title>
	<Alert.Description>
		<p>{m.projects.unreadableHelp}</p>
		<ul class="mt-2 flex flex-col gap-1">
			{#each unreadable as file (file.file)}
				<li>
					<p class="font-mono text-xs font-medium break-all text-foreground">{file.file}</p>
					<p class="text-xs break-words">{file.reason}</p>
				</li>
			{/each}
		</ul>
	</Alert.Description>
</Alert.Root>
