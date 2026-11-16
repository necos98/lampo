<script lang="ts">
	import Download from '@lucide/svelte/icons/download';

	import TaskProgress from '#lib/components/common/TaskProgress.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { packages, tasks } from '#lib/stores/index.js';
	import { compareMinor } from '#lib/stores/createProject.svelte.js';

	const t = m.create;

	// Composer is needed by the Laravel Installer: offer to install it right here.
	const composerRows = $derived(packages.rows.filter((r) => r.kind === 'composer').sort((a, b) => compareMinor(a.series, b.series)));
	const composerInstalled = $derived(composerRows.some((r) => r.installed !== null));
	const composerRow = $derived(composerRows.at(-1) ?? null);
	const composerTask = $derived(composerRow ? tasks.forKey(packages.taskKey('composer', composerRow.series, null)) : null);
	const composerInstalling = $derived(composerTask !== null && composerTask.outcome === null);
	/** Also true before the task is tracked: blocks a second click. */
	const composerBusy = $derived(composerRow !== null && packages.isInstalling(composerRow));
</script>

{#if packages.installed.data && !composerInstalled}
	<div class="flex items-center gap-3 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm">
		<span class="flex-1">{t.composerMissing}</span>
		{#if composerInstalling && composerTask}
			<div class="w-48"><TaskProgress progress={composerTask.progress} oncancel={() => tasks.cancel(composerTask.taskId)} /></div>
		{:else if composerRow}
			<Button type="button" size="xs" variant="outline" disabled={composerBusy} onclick={() => packages.install(composerRow)}><Download />{t.installComposer}</Button>
		{/if}
	</div>
{/if}
