<script lang="ts">
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';

	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import type { AddressKind } from '#lib/projects/addressList.js';
	import { projects } from '#lib/stores/index.js';
	import type { ProjectDraft } from '#lib/stores/projectDraft.svelte.js';

	import SelectField from '../../common/SelectField.svelte';

	let { draft }: { draft: ProjectDraft } = $props();
	const list = $derived(draft.addresses);
	const t = m.projects.addresses;
	const kinds = (Object.keys(t.kinds) as AddressKind[]).map((value) => ({ value, label: t.kinds[value] }));

	/** Re-fetched per new row so a second loopback row does not get the first one's port. */
	const suggest = () => projects.suggestAddresses(draft.projectId);
</script>

<!-- Above the list: a row added below would move the button out from under a double click. -->
<div class="mb-3 flex items-start justify-between gap-4">
	<p class="text-sm text-muted-foreground">{t.help}</p>
	<Button variant="secondary" size="sm" class="shrink-0" onclick={() => list.add('localhost', suggest)}><Plus />{t.add}</Button>
</div>
<ul class="flex flex-col gap-2">
	{#each list.rows as { id, address }, i (id)}
		<li class="flex items-center gap-2">
			<SelectField value={address.kind} options={kinds} class="w-52" aria-label={t.kind} onchange={(v) => list.changeKind(id, v as AddressKind, suggest)} />
			{#if address.kind === 'loopback-port'}
				<Input type="number" min="1" max="65535" class="h-8 w-28 font-mono text-xs" value={address.port} oninput={(e) => list.setPort(id, e.currentTarget.valueAsNumber)} aria-label={t.port} />
			{:else}
				<Input class="h-8 max-w-xs font-mono text-xs" value={address.host} oninput={(e) => list.setHost(id, e.currentTarget.value)} aria-label={t.host} />
			{/if}
			{#if address.kind === 'hostname'}<Badge variant="outline">{t.needsAdmin}</Badge>{/if}
			<Button variant="ghost" size="icon-xs" onclick={() => list.remove(id)} aria-label="{m.common.remove} {i + 1}" disabled={list.rows.length === 1}><X /></Button>
		</li>
	{/each}
</ul>
{#if list.rows.length === 0}<p class="text-sm text-destructive">{t.atLeastOne}</p>{/if}
