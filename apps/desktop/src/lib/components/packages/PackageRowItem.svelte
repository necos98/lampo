<script lang="ts">
	import ArrowUpCircle from '@lucide/svelte/icons/arrow-up-circle';
	import Download from '@lucide/svelte/icons/download';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';

	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import type { PackageRow } from '#lib/packages/grouping.js';
	import { describeError } from '#lib/services/errors.js';
	import { formatBytes } from '#lib/services/format.js';
	import { packages, tasks } from '#lib/stores/index.js';

	import ConfirmDialog from '../common/ConfirmDialog.svelte';
	import TaskProgress from '../common/TaskProgress.svelte';

	let { row }: { row: PackageRow } = $props();
	const t = m.packages;

	const kindLabel = $derived(t.kinds[row.kind] ?? row.kind);
	const title = $derived(row.phpVersion ? `${kindLabel} ${row.series}` : `${kindLabel} ${row.series}`);
	const task = $derived(tasks.forKey(packages.taskKey(row.kind, row.series, row.phpVersion)));
	/** A tracked task with progress to show; `installing` also covers the moment before it is tracked. */
	const running = $derived(task !== null && task.outcome === null);
	const installing = $derived(packages.isInstalling(row));
	const failed = $derived(task?.outcome?.status === 'failed' ? task.outcome.error : null);
	const vcMissing = $derived(row.latest.vcRuntime !== null && row.latest.vcRuntimeInstalled === false);
	const usedBy = $derived(row.installed?.usedBy ?? []);
	const uninstalling = $derived(row.installed !== null && packages.isUninstalling(row.installed.id));
	let confirming = $state(false);
</script>

<li class="rounded-md border bg-card px-3 py-2.5">
	<div class="grid grid-cols-[minmax(0,1fr)_7rem_auto] items-center gap-x-4">
		<div class="min-w-0">
			<p class="flex flex-wrap items-center gap-1.5 font-medium">
				{title}
				{#if row.phpVersion}<span class="text-xs font-normal text-muted-foreground">{t.forPhp(row.phpVersion)}</span>{/if}
				{#if row.endOfLife}<Badge variant="outline">{t.endOfLife}</Badge>{/if}
				{#if vcMissing}<Badge variant="destructive" title={t.vcRuntimeHelp(row.latest.vcRuntime ?? '')}><TriangleAlert class="size-3" />{t.vcRuntimeMissing}</Badge>{/if}
			</p>
			<p class="truncate font-mono text-xs text-muted-foreground">
				{#if row.installed}
					{row.installed.id.version}
					{#if row.update}<span class="text-amber-600 dark:text-amber-400"> → {row.update.latest}</span>{/if}
					{#if usedBy.length}<span class="font-sans"> · {t.inUse(usedBy)}</span>{/if}
				{:else}
					{row.latest.id.version} · {formatBytes(row.latest.sizeBytes)}
				{/if}
			</p>
		</div>
		<div>{#if row.installed}<Badge variant="secondary">{t.installed}</Badge>{/if}</div>
		<div class="flex items-center justify-end gap-1.5">
			{#if installing}
				<span class="text-xs text-muted-foreground">{t.installing}</span>
			{:else}
				{#if uninstalling}
					<span class="text-xs text-muted-foreground" role="status">{t.uninstalling}</span>
				{/if}
				{#if row.update}
					<Button size="xs" variant="secondary" disabled={uninstalling} onclick={() => packages.install(row)}><ArrowUpCircle />{t.updateTo(row.update.latest)}</Button>
				{/if}
				{#if row.installed}
					<Button size="icon-xs" variant="ghost" disabled={uninstalling || usedBy.length > 0} title={usedBy.length ? t.inUse(usedBy) : t.uninstall} aria-label="{t.uninstall} {title}" onclick={() => (confirming = true)}><Trash2 /></Button>
				{:else}
					<Button size="xs" onclick={() => packages.install(row)}><Download />{t.install}</Button>
				{/if}
			{/if}
		</div>
	</div>
	{#if running && task}
		<div class="mt-2"><TaskProgress progress={task.progress} oncancel={() => tasks.cancel(task.taskId)} /></div>
	{:else if failed && !installing}
		<p class="mt-2 text-xs text-destructive" role="alert">{t.failed} {describeError(failed)}</p>
	{/if}
</li>

{#if row.installed}
	<ConfirmDialog bind:open={confirming} title={t.uninstallTitle} body={t.uninstallBody(`${title} (${row.installed.id.version})`)} confirmLabel={t.uninstall} destructive onconfirm={() => packages.uninstall(row.installed!.id)} />
{/if}
