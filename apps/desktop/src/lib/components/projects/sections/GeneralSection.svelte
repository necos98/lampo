<script lang="ts">
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';

	import type { Project, TlsMode } from '#lib/api/index.js';
	import FormRow from '#lib/components/common/FormRow.svelte';
	import SelectField from '#lib/components/common/SelectField.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { presets, projects } from '#lib/stores/index.js';
	import type { ProjectDraft } from '#lib/stores/projectDraft.svelte.js';

	interface Props {
		draft: ProjectDraft;
		project: Project;
		onrelink: () => void;
	}

	let { draft, project, onrelink }: Props = $props();
	const g = m.projects.general;
	const tlsOptions = (Object.keys(g.tlsModes) as TlsMode[]).map((v) => ({ value: v, label: g.tlsModes[v] }));
	const origin = $derived(project.originPreset ? presets.list.data?.find((p) => p.id === project.originPreset) : undefined);
</script>

{#if !project.folderExists}
	<div class="mb-3 flex items-center gap-3 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm">
		<TriangleAlert class="size-4 shrink-0 text-amber-600" />
		<span class="flex-1">{m.projects.folderMissing}</span>
		<Button size="xs" variant="outline" onclick={onrelink}>{m.projects.relink}</Button>
	</div>
{/if}

<FormRow label={g.name} for="p-name"><Input id="p-name" class="h-8 max-w-sm" bind:value={draft.name} /></FormRow>
<FormRow label={g.root}>
	<div class="flex items-center gap-2">
		{#if project.folderExists}
			<button
				type="button"
				class="min-w-0 cursor-pointer truncate rounded bg-muted px-2 py-1 text-left font-mono text-xs underline-offset-2 hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
				title={m.projects.openFolder}
				onclick={() => projects.openFolder(project.id)}>{project.root}</button
			>
		{:else}
			<code class="truncate rounded bg-muted px-2 py-1 font-mono text-xs">{project.root}</code>
		{/if}
		<Button size="xs" variant="ghost" onclick={onrelink}>{m.projects.relink}</Button>
	</div>
</FormRow>
<FormRow label={g.publicDir} help={g.publicDirHelp}>
	{#snippet children(field)}
		<Input {...field} class="h-8 max-w-xs font-mono text-xs" bind:value={draft.environment.publicDir} placeholder="public" />
	{/snippet}
</FormRow>
<FormRow label={g.documentRoot}><code class="font-mono text-xs text-muted-foreground">{project.documentRoot}</code></FormRow>
<FormRow label={g.tls}>
	{#snippet children(field)}
		<SelectField {...field} value={draft.tls} options={tlsOptions} onchange={(v) => (draft.tls = v as TlsMode)} />
	{/snippet}
</FormRow>
{#if origin}
	<FormRow label={g.preset}><span class="text-sm">{presets.label(origin).name}</span></FormRow>
{/if}
