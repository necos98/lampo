<script lang="ts">
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import { onMount } from 'svelte';

	import { goto } from '$app/navigation';

	import ErrorNotice from '#lib/components/common/ErrorNotice.svelte';
	import PageHeader from '#lib/components/common/PageHeader.svelte';
	import ConfirmStep from '#lib/components/projects/create/ConfirmStep.svelte';
	import ExistingFolderStep from '#lib/components/projects/create/ExistingFolderStep.svelte';
	import ScaffoldRunStep from '#lib/components/projects/create/ScaffoldRunStep.svelte';
	import ScaffoldStep from '#lib/components/projects/create/ScaffoldStep.svelte';
	import SourcePicker from '#lib/components/projects/create/SourcePicker.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { createProjectFlow, packages, presets } from '#lib/stores/index.js';

	const flow = createProjectFlow();

	onMount(() => {
		// A clone or Laravel setup started before leaving this page continues where it got to.
		flow.resume();
		if (!presets.list.data) void presets.load();
		if (!packages.catalog.data) void packages.loadLocal();
		return () => flow.dispose();
	});

	const task = $derived(flow.task);

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (flow.source !== 'existing' && !flow.detection) {
			if (flow.phase === 'form' || flow.phase === 'failed') await flow.startScaffold();
			return;
		}
		if (!flow.detection) return void flow.detect();
		const project = await flow.create();
		if (project) await goto(`/projects/${project.id}`);
	}
</script>

<PageHeader title={m.create.title}>
	{#snippet actions()}
		<Button variant="ghost" size="sm" href="/projects"><ArrowLeft />{m.projects.backToList}</Button>
	{/snippet}
</PageHeader>

<form onsubmit={submit} class="flex max-w-2xl flex-col gap-4">
	<SourcePicker {flow} />

	{#if flow.source === 'existing'}
		<ExistingFolderStep {flow} />
	{:else}
		<ScaffoldStep {flow} />
		{#if flow.phase !== 'form' && task}<ScaffoldRunStep {flow} {task} />{/if}
		{#if flow.phase === 'done' && flow.detectError}<ErrorNotice error={flow.detectError} title={m.common.loadFailed} onretry={() => flow.detect()} />{/if}
	{/if}

	{#if flow.detection}<ConfirmStep {flow} detection={flow.detection} />{/if}
</form>
