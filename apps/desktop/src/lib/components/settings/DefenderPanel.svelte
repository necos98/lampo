<script lang="ts">
	import { onMount } from 'svelte';

	import { messages as m } from '#lib/messages/index.js';
	import { scanExclusion } from '#lib/stores/index.js';

	import AsyncBoundary from '../common/AsyncBoundary.svelte';
	import DefenderExclusionOption from './DefenderExclusionOption.svelte';
	import DefenderProjectsOption from './DefenderProjectsOption.svelte';

	const t = m.settings.defender;
	const info = $derived(scanExclusion.info.data);

	onMount(() => void scanExclusion.load());
</script>

<h2 class="text-sm font-medium">{t.title}</h2>
<p class="text-xs text-muted-foreground">{t.help}</p>
<AsyncBoundary status={scanExclusion.info.status} error={scanExclusion.info.error} onretry={scanExclusion.load} rows={4}>
	{#if info}
		<DefenderExclusionOption target="php" recommended paths={info.processes} confirmBody={t.php.confirmAdd.body} empty={t.php.none} />
		<DefenderExclusionOption target="mysql" paths={[info.folder]} confirmBody={t.mysql.confirmAdd.body(info.folder)} />
		<DefenderProjectsOption suggested={info.projectsFolder} />
		<p class="mt-4 text-xs text-muted-foreground">{t.unknownState}</p>
	{/if}
</AsyncBoundary>
