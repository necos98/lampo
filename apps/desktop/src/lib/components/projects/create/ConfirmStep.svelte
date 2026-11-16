<script lang="ts">
	import type { Detection } from '#lib/api/index.js';

	import FormRow from '#lib/components/common/FormRow.svelte';
	import SelectField from '#lib/components/common/SelectField.svelte';
	import AddressingChoice from '#lib/components/projects/AddressingChoice.svelte';
	import PhpVersionSelect from '#lib/components/projects/PhpVersionSelect.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { presets } from '#lib/stores/index.js';
	import type { CreateProjectFlow } from '#lib/stores/createProject.svelte.js';

	/** What detection found in the folder, and the choices to confirm before registering it. */
	let { flow, detection }: { flow: CreateProjectFlow; detection: Detection } = $props();
	const t = m.create;

	const presetOptions = $derived((presets.list.data ?? []).map((p) => ({ value: p.id, label: presets.label(p).name })));
	const detectedPreset = $derived(presets.list.data?.find((p) => p.id === detection.presetId));
</script>

<Card.Root size="sm">
	<Card.Header>
		<Card.Title>{flow.source === 'existing' ? t.stepConfirm : t.stepConfirmNew}</Card.Title>
		<Card.Description>
			{#if detection.projectType}{t.detected}: <Badge variant="secondary">{detectedPreset ? presets.label(detectedPreset).name : detection.projectType}</Badge>
			{:else}{t.notDetected}{/if}
		</Card.Description>
	</Card.Header>
	<Card.Content class="flex flex-col gap-1">
		{#if detection.alreadyRegistered}
			<div class="mb-2 flex items-center gap-3 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm">
				<span class="flex-1">{t.alreadyRegistered(detection.alreadyRegistered)}</span>
				<Button size="xs" variant="outline" href="/projects/{detection.alreadyRegistered}">{t.openExisting}</Button>
			</div>
		{/if}
		{#if detection.evidence.length}
			<FormRow label={t.evidence}>
				<ul class="flex flex-wrap gap-1.5">
					{#each detection.evidence as item (item)}<li><code class="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">{item}</code></li>{/each}
				</ul>
			</FormRow>
		{/if}
		{#if detection.apps?.length}
			<FormRow label={t.composeApps}>
				<p class="text-sm text-muted-foreground">{t.composeAppsHelp}</p>
				<ul class="flex flex-col gap-1 text-sm">
					{#each detection.apps as app (app.service)}<li><span class="font-medium">{app.service}</span>: <code class="font-mono text-xs">{t.composeApp(app.folder, app.phpVersion, app.documentRoot)}</code></li>{/each}
				</ul>
			</FormRow>
		{/if}
		{#if detection.warnings.length}
			<FormRow label={t.warnings}>
				<ul class="flex flex-col gap-1 text-sm text-amber-700 dark:text-amber-400">
					{#each detection.warnings as item (item)}<li>{item}</li>{/each}
				</ul>
			</FormRow>
		{/if}
		<FormRow label={t.name} for="name">
			<Input id="name" class="h-8 max-w-xs" bind:value={flow.name} />
		</FormRow>
		<FormRow label={t.preset}>
			{#snippet children(field)}
				<SelectField {...field} bind:value={flow.presetId} options={presetOptions} />
			{/snippet}
		</FormRow>
		<FormRow label={t.php}>
			{#snippet children(field)}
				<PhpVersionSelect {...field} bind:value={flow.phpVersion} allowDefault />
			{/snippet}
		</FormRow>
		<AddressingChoice legend={t.addressing} help={t.addressingHelp} value={flow.addressing} disabled={flow.creating} onchange={(mode) => (flow.addressing = mode)} />
	</Card.Content>
	<Card.Footer class="justify-end">
		<Button type="submit" size="sm" disabled={!flow.canCreate}>{flow.creating ? t.creating : t.confirm}</Button>
	</Card.Footer>
</Card.Root>
