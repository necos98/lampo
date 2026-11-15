<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';

	import type { AppError } from '#lib/api/index.js';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { describeError, errorOutput } from '#lib/services/errors.js';

	interface Props {
		error: AppError | null;
		title?: string;
		onretry?: () => void;
	}

	let { error, title = m.common.loadFailed, onretry }: Props = $props();
	const output = $derived(error ? errorOutput(error) : null);
</script>

<Alert.Root variant="destructive" role="alert">
	<CircleAlert />
	<Alert.Title>{title}</Alert.Title>
	<Alert.Description>
		<p>{error ? describeError(error) : ''}</p>
		{#if output}
			<pre class="mt-2 max-h-40 overflow-auto rounded bg-muted p-2 font-mono text-xs whitespace-pre-wrap text-foreground">{output}</pre>
		{/if}
		{#if onretry}
			<Button variant="outline" size="xs" class="mt-2" onclick={onretry}>{m.common.retry}</Button>
		{/if}
	</Alert.Description>
</Alert.Root>
