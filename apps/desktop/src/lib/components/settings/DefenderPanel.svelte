<script lang="ts">
	import { onMount } from 'svelte';

	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { scanExclusion } from '#lib/stores/index.js';
	import { scanExclusionMessage } from '#lib/stores/scanExclusion.svelte.js';

	import AsyncBoundary from '../common/AsyncBoundary.svelte';
	import ConfirmDialog from '../common/ConfirmDialog.svelte';
	import CopyField from '../common/CopyField.svelte';

	const t = m.settings.defender;
	const info = $derived(scanExclusion.info.data);
	const busy = $derived(scanExclusion.busy !== null);
	const last = $derived(scanExclusion.last);
	let confirmAdd = $state(false);

	onMount(() => void scanExclusion.load());
</script>

<h2 class="text-sm font-medium">{t.title}</h2>
<p class="text-xs text-muted-foreground">{t.help}</p>
<p class="mt-1 mb-3 text-xs text-muted-foreground">{t.risk}</p>
<AsyncBoundary status={scanExclusion.info.status} error={scanExclusion.info.error} onretry={scanExclusion.load} rows={2}>
	{#if info}
		<CopyField label={t.folder} value={info.folder} />
		<div class="mt-2 flex flex-wrap items-center justify-end gap-2">
			<Button size="sm" variant="outline" disabled={busy} onclick={() => scanExclusion.remove()}>
				{scanExclusion.busy === 'remove' ? t.removing : t.remove}
			</Button>
			<Button size="sm" variant="outline" disabled={busy} onclick={() => (confirmAdd = true)}>
				{scanExclusion.busy === 'add' ? t.adding : t.add}
			</Button>
		</div>
		{#if last}
			<p class="mt-2 text-xs" role="status">
				<span class="text-muted-foreground">{t.last}:</span>
				{scanExclusionMessage(last)}
			</p>
		{/if}
		<p class="mt-2 text-xs text-muted-foreground">{t.unknownState}</p>

		<ConfirmDialog
			bind:open={confirmAdd}
			title={t.confirmAdd.title}
			body={t.confirmAdd.body(info.folder)}
			confirmLabel={t.confirmAdd.confirm}
			onconfirm={scanExclusion.add}
		/>
	{/if}
</AsyncBoundary>
