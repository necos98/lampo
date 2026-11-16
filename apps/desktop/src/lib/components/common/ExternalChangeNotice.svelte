<script lang="ts">
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';

	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';

	/** Shown while a draft has edits and the backend value it was loaded from changed under it. */
	let { onreload, onoverwrite }: { onreload: () => void; onoverwrite: () => void } = $props();
	const t = m.unsaved;
</script>

<Alert.Root role="status" class="mb-4">
	<TriangleAlert />
	<Alert.Title>{t.changedElsewhereTitle}</Alert.Title>
	<Alert.Description>
		<p>{t.changedElsewhereBody}</p>
		<div class="mt-2 flex gap-2">
			<Button size="sm" variant="outline" onclick={onreload}>{t.reload}</Button>
			<Button size="sm" variant="ghost" onclick={onoverwrite}>{t.overwrite}</Button>
		</div>
	</Alert.Description>
</Alert.Root>
