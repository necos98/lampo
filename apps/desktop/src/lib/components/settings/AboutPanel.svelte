<script lang="ts">
	import { onMount } from 'svelte';

	import { messages as m } from '#lib/messages/index.js';
	import { diagnostics } from '#lib/stores/index.js';

	import CopyField from '../common/CopyField.svelte';

	const info = $derived(diagnostics.info.data);

	onMount(() => void diagnostics.info.load());
</script>

<h2 class="mb-2 text-sm font-medium">{m.settings.about.title}</h2>
{#if info}
	<div class="flex flex-col gap-1 pb-6">
		<CopyField label={m.app.version} value={info.version} />
		<CopyField label={m.settings.about.config} value={info.configDir} />
		<CopyField label={m.settings.about.data} value={info.dataDir} />
	</div>
{/if}
