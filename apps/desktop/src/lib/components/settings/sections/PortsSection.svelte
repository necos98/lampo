<script lang="ts">
	import FormRow from '#lib/components/common/FormRow.svelte';
	import { Input } from '#lib/components/ui/input/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import type { SettingsDraft } from '#lib/stores/settingsDraft.svelte.js';

	let { draft }: { draft: SettingsDraft } = $props();
	const t = m.settings;

	const num = (e: Event & { currentTarget: HTMLInputElement }) => e.currentTarget.valueAsNumber;
</script>

<h2 class="mb-1 text-sm font-medium">{t.ports}</h2>
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
