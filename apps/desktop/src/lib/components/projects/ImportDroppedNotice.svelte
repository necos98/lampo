<script lang="ts">
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';

	import type { ImportProjectResult } from '#lib/api/index.js';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { messages as m } from '#lib/messages/index.js';

	/** What an import left out of the file (`droppedApacheDirectives`, `droppedPhpIniKeys`). */
	let { result }: { result: ImportProjectResult } = $props();
</script>

<Alert.Root role="status">
	<TriangleAlert />
	<Alert.Title>{m.projects.importDroppedTitle}</Alert.Title>
	<Alert.Description>
		<p>{m.projects.importDroppedHelp}</p>
		{#if result.droppedPhpIniKeys.length > 0}
			<p class="mt-2 font-medium text-foreground">{m.projects.importDroppedIni}</p>
			<p class="font-mono text-xs">{result.droppedPhpIniKeys.join(', ')}</p>
		{/if}
		{#if result.droppedApacheDirectives}
			<p class="mt-2 font-medium text-foreground">{m.projects.importDroppedDirectives}</p>
			<pre class="max-h-40 overflow-auto rounded bg-muted p-2 font-mono text-xs whitespace-pre-wrap text-foreground">{result.droppedApacheDirectives}</pre>
		{/if}
	</Alert.Description>
</Alert.Root>
