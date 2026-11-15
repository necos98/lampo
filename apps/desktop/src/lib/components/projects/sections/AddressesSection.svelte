<script lang="ts">
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';

	import type { SiteAddress } from '#lib/api/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { projects } from '#lib/stores/index.js';
	import type { ProjectDraft } from '#lib/stores/projectDraft.svelte.js';

	import SelectField from '../../common/SelectField.svelte';

	let { draft }: { draft: ProjectDraft } = $props();
	const t = m.projects.addresses;
	const kinds = (Object.keys(t.kinds) as SiteAddress['kind'][]).map((value) => ({ value, label: t.kinds[value] }));

	/** Suggestions from the backend (domain rules + port allocator); re-fetched per new row so a
	 * second loopback row does not get the first one's port. */
	async function blank(kind: SiteAddress['kind']): Promise<SiteAddress> {
		const s = await projects.suggestAddresses(draft.projectId);
		if (kind === 'hostname') return s?.hostname ?? { kind, host: '' };
		if (kind === 'localhost') return s?.localhost ?? { kind, host: '' };
		const taken = new Set(draft.addresses.flatMap((a) => (a.kind === 'loopback-port' ? [a.port] : [])));
		const suggested = s?.loopbackPort.kind === 'loopback-port' ? s.loopbackPort.port : NaN;
		// The allocator only knows saved sites: avoid a port already used by an unsaved row.
		return { kind, port: taken.has(suggested) ? NaN : suggested };
	}

	async function changeKind(index: number, kind: SiteAddress['kind']) {
		if (draft.addresses[index].kind !== kind) draft.addresses[index] = await blank(kind);
	}

	async function add() {
		draft.addresses.push(await blank('localhost'));
	}

	function setText(index: number, host: string) {
		const a = draft.addresses[index];
		if (a.kind !== 'loopback-port') draft.addresses[index] = { ...a, host };
	}

	function setPort(index: number, port: number) {
		draft.addresses[index] = { kind: 'loopback-port', port };
	}
</script>

<p class="mb-3 text-sm text-muted-foreground">{t.help}</p>
<ul class="flex flex-col gap-2">
	{#each draft.addresses as address, i (i)}
		<li class="flex items-center gap-2">
			<SelectField value={address.kind} options={kinds} class="w-52" onchange={(v) => changeKind(i, v as SiteAddress['kind'])} />
			{#if address.kind === 'loopback-port'}
				<Input type="number" min="1" max="65535" class="h-8 w-28 font-mono text-xs" value={address.port} oninput={(e) => setPort(i, e.currentTarget.valueAsNumber)} aria-label={t.port} />
			{:else}
				<Input class="h-8 max-w-xs font-mono text-xs" value={address.host} oninput={(e) => setText(i, e.currentTarget.value)} aria-label={t.host} />
			{/if}
			{#if address.kind === 'hostname'}<Badge variant="outline">{t.needsAdmin}</Badge>{/if}
			<Button variant="ghost" size="icon-xs" onclick={() => draft.addresses.splice(i, 1)} aria-label="{m.common.remove} {i + 1}" disabled={draft.addresses.length === 1}><X /></Button>
		</li>
	{/each}
</ul>
{#if draft.addresses.length === 0}<p class="text-sm text-destructive">{t.atLeastOne}</p>{/if}
<Button variant="secondary" size="sm" class="mt-3" onclick={add}><Plus />{t.add}</Button>
