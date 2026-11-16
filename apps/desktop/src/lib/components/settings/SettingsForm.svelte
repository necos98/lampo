<script lang="ts">
	import { onMount, untrack } from 'svelte';

	import type { Settings } from '#lib/api/index.js';
	import ExternalChangeNotice from '#lib/components/common/ExternalChangeNotice.svelte';
	import FormRow from '#lib/components/common/FormRow.svelte';
	import SaveBar from '#lib/components/common/SaveBar.svelte';
	import SelectField from '#lib/components/common/SelectField.svelte';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { packages, settings, unsavedChanges } from '#lib/stores/index.js';
	import { SettingsDraft } from '#lib/stores/settingsDraft.svelte.js';

	let { source }: { source: Settings } = $props();
	const t = m.settings;

	const draft = new SettingsDraft(untrack(() => source));

	// Backend changes flow in unless the user is mid-edit; then the notice asks what to keep.
	const changedElsewhere = $derived(!settings.saving && draft.dirty && draft.isStale(source));
	$effect(() => {
		if (!draft.dirty && draft.isStale(source)) draft.reset(source);
	});

	onMount(() =>
		unsavedChanges.register({
			get dirty() {
				return draft.dirty;
			},
			save,
			discard
		})
	);

	const phpOptions = $derived(packages.phpOptions.filter((o) => o.installed).map((o) => ({ value: o.series, label: `PHP ${o.series}` })));
	const phpWithCurrent = $derived(draft.value.defaultPhpVersion && !phpOptions.some((o) => o.value === draft.value.defaultPhpVersion) ? [...phpOptions, { value: draft.value.defaultPhpVersion, label: `PHP ${draft.value.defaultPhpVersion}` }] : phpOptions);

	function discard() {
		draft.reset(source);
	}

	async function save() {
		if (settings.saving || !draft.dirty || !draft.valid) return;
		const result = await settings.save(draft.toSettings());
		if (result.ok) draft.reset(result.value);
	}

	const num = (e: Event & { currentTarget: HTMLInputElement }) => e.currentTarget.valueAsNumber;
</script>

{#if changedElsewhere}
	<ExternalChangeNotice onreload={discard} onoverwrite={() => draft.rebase(source)} />
{/if}

<h2 class="mb-1 text-sm font-medium">{t.defaults}</h2>
<FormRow label={t.defaultPhp} help={t.defaultPhpHelp}>
	{#snippet children(field)}
		<SelectField {...field} value={draft.value.defaultPhpVersion ?? ''} options={phpWithCurrent} onchange={(v) => (draft.value.defaultPhpVersion = v)} placeholder={m.common.none} />
	{/snippet}
</FormRow>
<FormRow label={t.tld} help={t.tldHelp}>
	{#snippet children(field)}
		<Input {...field} class="h-8 w-32 font-mono text-xs" bind:value={draft.value.tld} />
	{/snippet}
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
			{#if instance.uiPort != null}
				<!-- Kinds with a second listener (Mailpit's web UI) edit it right below. -->
				<li class="flex items-center gap-3">
					<span class="w-24 text-xs text-muted-foreground">{instance.label} · {t.uiPort}</span>
					<Input
						type="number"
						min="1"
						max="65535"
						class="h-8 w-28 font-mono text-xs"
						value={instance.uiPort}
						oninput={(e) => (draft.value.services[i].uiPort = num(e))}
						aria-label="{m.common.port} {instance.label} {t.uiPort}"
					/>
				</li>
			{/if}
		{/each}
	</ul>
</FormRow>
<FormRow label={t.worktreeRange} help={draft.rangeValid ? t.worktreeRangeHelp : t.rangeInvalid}>
	{#snippet children(field)}
		<div class="flex items-center gap-2">
			<Input {...field} type="number" class="h-8 w-24 font-mono text-xs" value={draft.value.loopbackPortRange.start} oninput={(e) => (draft.value.loopbackPortRange.start = num(e))} aria-label={t.rangeStart} aria-invalid={!draft.rangeValid} />
			<span class="text-muted-foreground">–</span>
			<Input type="number" class="h-8 w-24 font-mono text-xs" value={draft.value.loopbackPortRange.end} oninput={(e) => (draft.value.loopbackPortRange.end = num(e))} aria-label={t.rangeEnd} aria-invalid={!draft.rangeValid} aria-describedby={field['aria-describedby']} />
		</div>
	{/snippet}
</FormRow>

<h2 class="mt-5 mb-1 text-sm font-medium">{t.behaviour}</h2>
<FormRow label={t.closeToTray} help={t.closeToTrayHelp}>
	{#snippet children(field)}
		<Switch {...field} bind:checked={draft.value.closeToTray} />
	{/snippet}
</FormRow>

<SaveBar dirty={draft.dirty} valid={draft.valid} saving={settings.saving} onsave={save} onreset={discard} />
