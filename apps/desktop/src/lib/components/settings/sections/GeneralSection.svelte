<script lang="ts">
	import { onMount } from 'svelte';

	import type { Addressing } from '#lib/api/index.js';
	import FormRow from '#lib/components/common/FormRow.svelte';
	import SelectField from '#lib/components/common/SelectField.svelte';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { ADDRESSING_MODES } from '#lib/projects/addressing.js';
	import { packages } from '#lib/stores/index.js';
	import type { SettingsDraft } from '#lib/stores/settingsDraft.svelte.js';

	let { draft }: { draft: SettingsDraft } = $props();
	const t = m.settings;
	const addressingOptions = ADDRESSING_MODES.map((mode) => ({ value: mode, label: m.addressing.modes[mode].title }));

	const phpOptions = $derived(packages.phpOptions.filter((o) => o.installed).map((o) => ({ value: o.series, label: `PHP ${o.series}` })));
	const phpWithCurrent = $derived(draft.value.defaultPhpVersion && !phpOptions.some((o) => o.value === draft.value.defaultPhpVersion) ? [...phpOptions, { value: draft.value.defaultPhpVersion, label: `PHP ${draft.value.defaultPhpVersion}` }] : phpOptions);

	onMount(() => {
		if (!packages.catalog.data) void packages.loadLocal();
	});
</script>

<h2 class="mb-1 text-sm font-medium">{t.defaults}</h2>
<FormRow label={t.defaultPhp} help={t.defaultPhpHelp}>
	{#snippet children(field)}
		<SelectField {...field} value={draft.value.defaultPhpVersion ?? ''} options={phpWithCurrent} onchange={(v) => (draft.value.defaultPhpVersion = v)} placeholder={m.common.none} />
	{/snippet}
</FormRow>
<FormRow label={t.defaultAddressing} help={m.addressing.modes[draft.value.defaultAddressing].description}>
	{#snippet children(field)}
		<SelectField {...field} value={draft.value.defaultAddressing} options={addressingOptions} onchange={(v) => (draft.value.defaultAddressing = v as Addressing)} />
	{/snippet}
</FormRow>
<FormRow label={t.tld} help={t.tldHelp}>
	{#snippet children(field)}
		<Input {...field} class="h-8 w-32 font-mono text-xs" bind:value={draft.value.tld} />
	{/snippet}
</FormRow>

<h2 class="mt-5 mb-1 text-sm font-medium">{t.behaviour}</h2>
<FormRow label={t.closeToTray} help={t.closeToTrayHelp}>
	{#snippet children(field)}
		<Switch {...field} bind:checked={draft.value.closeToTray} />
	{/snippet}
</FormRow>
<FormRow label={t.autoRestart} help={t.autoRestartHelp}>
	{#snippet children(field)}
		<Switch {...field} bind:checked={draft.value.autoRestartServices} />
	{/snippet}
</FormRow>
