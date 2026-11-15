<script lang="ts">
	import { untrack } from 'svelte';

	import type { Settings } from '#lib/api/index.js';
	import FormRow from '#lib/components/common/FormRow.svelte';
	import SaveBar from '#lib/components/common/SaveBar.svelte';
	import SelectField from '#lib/components/common/SelectField.svelte';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { packages, settings } from '#lib/stores/index.js';
	import { SettingsDraft } from '#lib/stores/settingsDraft.svelte.js';

	let { source }: { source: Settings } = $props();
	const t = m.settings;

	const draft = new SettingsDraft(untrack(() => source));

	$effect(() => {
		const latest = source;
		untrack(() => {
			if (!draft.dirty) draft.reset(latest);
		});
	});

	const phpOptions = $derived(packages.phpOptions.filter((o) => o.installed).map((o) => ({ value: o.series, label: `PHP ${o.series}` })));
	const phpWithCurrent = $derived(draft.value.defaultPhpVersion && !phpOptions.some((o) => o.value === draft.value.defaultPhpVersion) ? [...phpOptions, { value: draft.value.defaultPhpVersion, label: `PHP ${draft.value.defaultPhpVersion}` }] : phpOptions);

	async function save() {
		const result = await settings.save(draft.toSettings());
		if (result.ok) draft.reset(result.value);
	}

	const num = (e: Event & { currentTarget: HTMLInputElement }) => e.currentTarget.valueAsNumber;
</script>

<h2 class="mb-1 text-sm font-medium">{t.defaults}</h2>
<FormRow label={t.defaultPhp} for="default-php" help={t.defaultPhpHelp}>
	<SelectField id="default-php" value={draft.value.defaultPhpVersion ?? ''} options={phpWithCurrent} onchange={(v) => (draft.value.defaultPhpVersion = v)} placeholder={m.common.none} />
</FormRow>
<FormRow label={t.tld} for="tld" help={t.tldHelp}>
	<Input id="tld" class="h-8 w-32 font-mono text-xs" bind:value={draft.value.tld} />
</FormRow>

<h2 class="mt-5 mb-1 text-sm font-medium">{t.ports}</h2>
<FormRow label={t.apachePort} for="apache-port">
	<Input id="apache-port" type="number" min="1" max="65535" class="h-8 w-28 font-mono text-xs" value={draft.value.apachePort} oninput={(e) => (draft.value.apachePort = num(e))} />
</FormRow>
<FormRow label={t.serviceInstances}>
	<ul class="flex flex-col gap-1.5">
		{#each draft.value.services as instance, i (instance.service)}
			<li class="flex items-center gap-3">
				<span class="w-24 text-sm">{instance.label}</span>
				<Input type="number" min="1" max="65535" class="h-8 w-28 font-mono text-xs" value={instance.port} oninput={(e) => (draft.value.services[i].port = num(e))} aria-label="{m.common.port} {instance.label}" />
				<span class="text-xs text-muted-foreground">{instance.alwaysOn ? t.alwaysOn : t.onDemand}</span>
			</li>
		{/each}
	</ul>
</FormRow>
<FormRow label={t.worktreeRange} help={draft.rangeValid ? t.worktreeRangeHelp : t.rangeInvalid}>
	<div class="flex items-center gap-2">
		<Input type="number" class="h-8 w-24 font-mono text-xs" value={draft.value.loopbackPortRange.start} oninput={(e) => (draft.value.loopbackPortRange.start = num(e))} aria-label={t.rangeStart} aria-invalid={!draft.rangeValid} />
		<span class="text-muted-foreground">–</span>
		<Input type="number" class="h-8 w-24 font-mono text-xs" value={draft.value.loopbackPortRange.end} oninput={(e) => (draft.value.loopbackPortRange.end = num(e))} aria-label={t.rangeEnd} aria-invalid={!draft.rangeValid} />
	</div>
</FormRow>

<h2 class="mt-5 mb-1 text-sm font-medium">{t.behaviour}</h2>
<FormRow label={t.closeToTray} help={t.closeToTrayHelp}>
	<Switch bind:checked={draft.value.closeToTray} aria-label={t.closeToTray} />
</FormRow>

<SaveBar dirty={draft.dirty} valid={draft.valid} saving={settings.saving} onsave={save} onreset={() => draft.reset(source)} />
