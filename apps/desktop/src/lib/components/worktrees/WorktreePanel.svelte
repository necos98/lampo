<script lang="ts">
	import Plus from '@lucide/svelte/icons/plus';
	import Sparkles from '@lucide/svelte/icons/sparkles';
	import X from '@lucide/svelte/icons/x';

	import type { WorktreeEnvironment } from '#lib/api/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Checkbox } from '#lib/components/ui/checkbox/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { describeError } from '#lib/services/errors.js';
	import { notifier } from '#lib/services/notifier.js';
	import { tasks, worktrees } from '#lib/stores/index.js';

	import AsyncBoundary from '../common/AsyncBoundary.svelte';
	import ConfirmDialog from '../common/ConfirmDialog.svelte';
	import TaskProgress from '../common/TaskProgress.svelte';
	import CreateWorktreeDialog from './CreateWorktreeDialog.svelte';
	import WorktreeRow from './WorktreeRow.svelte';

	let { projectId }: { projectId: string } = $props();
	const t = m.worktrees;

	const items = $derived(worktrees.forProject(projectId));
	const staleCount = $derived(items.filter((w) => w.stale).length);

	let creating = $state(false);
	const running = $derived(worktrees.creations(projectId));

	let removing = $state<WorktreeEnvironment | null>(null);
	let removeOpen = $state(false);
	let dropDatabase = $state(false);
	let pruneList = $state<string[]>([]);
	let pruneOpen = $state(false);

	function askRemove(w: WorktreeEnvironment) {
		removing = w;
		dropDatabase = false;
		removeOpen = true;
	}

	const confirmRemove = () => (removing ? worktrees.remove(removing.site, dropDatabase) : null);

	async function askPrune() {
		const preview = await worktrees.preview(projectId);
		if (!preview.ok) return;
		if (preview.value.removed.length === 0) return void notifier.info(t.pruneNone);
		pruneList = preview.value.removed;
		pruneOpen = true;
	}
</script>

<div class="mb-3 flex items-center justify-between gap-3">
	<p class="text-sm text-muted-foreground">{t.subtitle}</p>
	<div class="flex shrink-0 gap-2">
		{#if staleCount > 0}
			<Button variant="outline" size="sm" onclick={askPrune}><Sparkles />{t.prune}</Button>
		{/if}
		<Button size="sm" onclick={() => (creating = true)}><Plus />{t.create}</Button>
	</div>
</div>

<AsyncBoundary status={worktrees.list.status} error={worktrees.list.error} onretry={worktrees.load} empty={items.length === 0 && running.length === 0} emptyMessage={t.empty} rows={2}>
	<ul class="flex flex-col gap-1.5">
		{#each running as { path, task } (path)}
			<li class="rounded-md border border-dashed px-3 py-2">
				<div class="mb-1 flex items-center justify-between gap-2">
					<p class="truncate font-mono text-xs">{path}</p>
					{#if task.outcome}
						<Button variant="ghost" size="icon-xs" onclick={() => worktrees.dismissCreation(projectId, path)} aria-label={t.dismissCreation(path)} title={m.common.close}><X /></Button>
					{/if}
				</div>
				{#if task.outcome?.status === 'failed'}
					<p class="text-sm text-destructive">{t.createFailed} {describeError(task.outcome.error)}</p>
				{:else if task.outcome?.status === 'cancelled'}
					<p class="text-sm text-muted-foreground">{m.errors.cancelled}</p>
				{:else}
					<TaskProgress progress={task.progress} oncancel={() => tasks.cancel(task.taskId)} />
				{/if}
			</li>
		{/each}
		{#each items as worktree (worktree.site)}
			<WorktreeRow {worktree} onremove={askRemove} />
		{/each}
	</ul>
</AsyncBoundary>

<CreateWorktreeDialog bind:open={creating} {projectId} />

<ConfirmDialog bind:open={removeOpen} title={t.removeTitle} body={removing ? t.removeBody(removing.site) : ''} confirmLabel={t.remove} destructive onconfirm={confirmRemove}>
	{#if removing?.database && removing.databaseMode.mode !== 'shared'}
		<div class="flex items-center gap-2 px-1">
			<Checkbox id="drop-db" bind:checked={dropDatabase} />
			<Label for="drop-db" class="text-sm font-normal">{t.removeDropDatabase}</Label>
		</div>
	{/if}
</ConfirmDialog>

<ConfirmDialog bind:open={pruneOpen} title={t.pruneTitle} body="{t.pruneBody} {pruneList.join(', ')}" confirmLabel={t.prune} destructive onconfirm={() => worktrees.prune(projectId, false)} />
