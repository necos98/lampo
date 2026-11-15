<script lang="ts">
	import * as Select from '#lib/components/ui/select/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { logSources } from '#lib/stores/index.js';
	import { groupOf } from '#lib/stores/logSources.svelte.js';

	let { value, onchange }: { value: string | null; onchange: (id: string) => void } = $props();

	/** `error:shop` -> `shop`; service sources keep their id. */
	const shortName = (id: string) => (id.includes(':') ? id.split(':').slice(1).join(':') : id);
	const groups = $derived(logSources.grouped());
	const currentLabel = $derived.by(() => {
		const source = (logSources.list.data ?? []).find((s) => s.id === value);
		if (!source) return m.logs.pickSource;
		const group = groupOf(source);
		return group === 'services' ? source.id : `${m.logs.groups[group]}: ${shortName(source.id)}`;
	});
</script>

<Select.Root type="single" value={value ?? ''} onValueChange={onchange}>
	<Select.Trigger size="sm" class="w-64" aria-label={m.logs.source}>{currentLabel}</Select.Trigger>
	<Select.Content>
		{#each groups as g (g.group)}
			<Select.Group>
				<Select.GroupHeading>{m.logs.groups[g.group]}</Select.GroupHeading>
				{#each g.sources as source (source.id)}
					<Select.Item value={source.id} label={shortName(source.id)}>
						{shortName(source.id)}{source.exists ? '' : ` · ${m.logs.missingFile}`}
					</Select.Item>
				{/each}
			</Select.Group>
		{/each}
	</Select.Content>
</Select.Root>
