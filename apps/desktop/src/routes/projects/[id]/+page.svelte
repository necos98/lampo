<script lang="ts">
	import { onMount } from 'svelte';

	import { page } from '$app/state';

	import AsyncBoundary from '#lib/components/common/AsyncBoundary.svelte';
	import EmptyState from '#lib/components/common/EmptyState.svelte';
	import ProjectDetail from '#lib/components/projects/ProjectDetail.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { packages, projects } from '#lib/stores/index.js';

	const id = $derived(page.params.id ?? '');
	const project = $derived(projects.get(id));

	onMount(() => {
		// The PHP picker needs the catalog; projects are already loaded by the shell.
		if (!packages.catalog.data) void packages.loadLocal();
	});
</script>

<AsyncBoundary status={projects.list.status} error={projects.list.error} onretry={projects.load} rows={3}>
	{#if project}
		{#key project.id}
			<ProjectDetail {project} />
		{/key}
	{:else}
		<EmptyState message={m.projects.notFound}>
			<Button size="sm" variant="outline" href="/projects">{m.projects.backToList}</Button>
		</EmptyState>
	{/if}
</AsyncBoundary>
