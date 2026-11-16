<script lang="ts">
	import Copy from '@lucide/svelte/icons/copy';
	import FileDown from '@lucide/svelte/icons/file-down';
	import FileText from '@lucide/svelte/icons/file-text';
	import FolderOpen from '@lucide/svelte/icons/folder-open';

	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { logFiles } from '#lib/stores/index.js';
	import type { LogSession } from '#lib/stores/logSession.svelte.js';

	let { session }: { session: LogSession } = $props();

	const t = m.logs;
	let exporting = $state(false);
	const visibleTexts = () => session.visible.map((l) => l.text);
	const empty = $derived(session.visible.length === 0);

	async function exportVisible(source: string): Promise<void> {
		exporting = true;
		try {
			await logFiles.export(source, visibleTexts());
		} finally {
			exporting = false;
		}
	}
</script>

{#if session.source}
	{@const source = session.source}
	<div class="flex items-center gap-1">
		<Button variant="ghost" size="sm" title={t.copyHelp} disabled={empty} onclick={() => logFiles.copy(visibleTexts())}><Copy />{t.copy}</Button>
		<Button variant="ghost" size="sm" title={t.exportHelp} disabled={empty || exporting} onclick={() => exportVisible(source)}><FileDown />{t.export}</Button>
		<Button variant="ghost" size="sm" title={t.openFileHelp} onclick={() => logFiles.openFile(source)}><FileText />{t.openFile}</Button>
		<Button variant="ghost" size="sm" title={t.openFolderHelp} onclick={() => logFiles.openFolder(source)}><FolderOpen />{t.openFolder}</Button>
	</div>
{/if}
