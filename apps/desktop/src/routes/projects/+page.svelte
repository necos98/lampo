<script lang="ts">
	import Download from '@lucide/svelte/icons/download';
	import Plus from '@lucide/svelte/icons/plus';

	import AsyncBoundary from '#lib/components/common/AsyncBoundary.svelte';
	import PageHeader from '#lib/components/common/PageHeader.svelte';
	import ImportProjectDialog from '#lib/components/projects/ImportProjectDialog.svelte';
	import ProjectRow from '#lib/components/projects/ProjectRow.svelte';
	import UnreadableProjectsNotice from '#lib/components/projects/UnreadableProjectsNotice.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { filterProjects } from '#lib/projects/projectFilter.js';
	import { projects } from '#lib/stores/index.js';

	let filter = $state('');
	let importing = $state(false);

	const all = $derived(projects.all);
	const visible = $derived(filterProjects(all, filter));
</script>

<PageHeader title={m.projects.title} subtitle={projects.list.data ? m.projects.subtitle(all.length) : undefined}>
	{#snippet actions()}
		<Button variant="outline" size="sm" onclick={() => (importing = true)}><Download />{m.projects.import}</Button>
		<Button size="sm" href="/projects/new"><Plus />{m.projects.add}</Button>
	{/snippet}
</PageHeader>

{#if projects.unreadable.length > 0}
	<UnreadableProjectsNotice unreadable={projects.unreadable} />
{/if}

{#if all.length > 0}
	<Input data-filter class="mb-3 h-8 max-w-xs" placeholder={m.common.search} bind:value={filter} aria-label={m.common.search} />
{/if}

<AsyncBoundary status={projects.list.status} error={projects.list.error} onretry={projects.load} empty={all.length === 0} emptyMessage={m.projects.empty}>
	{#snippet emptyAction()}
		<Button size="sm" href="/projects/new"><Plus />{m.projects.add}</Button>
	{/snippet}
	{#if visible.length === 0}
		<p class="py-6 text-center text-sm text-muted-foreground">{m.projects.emptyFiltered}</p>
	{:else}
		<ul class="flex flex-col gap-1.5">
			{#each visible as project (project.id)}
				<ProjectRow {project} />
			{/each}
		</ul>
	{/if}
</AsyncBoundary>

<ImportProjectDialog bind:open={importing} />
