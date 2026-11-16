<script lang="ts">
	import Code from '@lucide/svelte/icons/code';
	import FolderOpen from '@lucide/svelte/icons/folder-open';
	import ScrollText from '@lucide/svelte/icons/scroll-text';
	import SquareTerminal from '@lucide/svelte/icons/square-terminal';

	import type { Project } from '#lib/api/index.js';
	import CopyButton from '#lib/components/common/CopyButton.svelte';
	import IconAction from '#lib/components/common/IconAction.svelte';
	import { messages as m } from '#lib/messages/index.js';
	import { projectLogsHref } from '#lib/projects/links.js';
	import { projects } from '#lib/stores/index.js';

	let { project, class: className = '' }: { project: Pick<Project, 'id' | 'name' | 'url' | 'folderExists'>; class?: string } = $props();
	const t = m.projects;
	const noFolder = $derived(!project.folderExists);
</script>

<div role="group" aria-label={t.rowActions(project.name)} class="flex items-center gap-0.5 {className}">
	<IconAction label={t.openTerminal} disabled={noFolder} onclick={() => projects.openTerminal(project.id)}><SquareTerminal /></IconAction>
	<IconAction label={t.openInVsCodeHelp} disabled={noFolder} onclick={() => projects.openInVsCode(project.id)}><Code /></IconAction>
	<IconAction label={t.openFolder} disabled={noFolder} onclick={() => projects.openFolder(project.id)}><FolderOpen /></IconAction>
	<IconAction label={t.openLogs} href={projectLogsHref(project.id)}><ScrollText /></IconAction>
	<CopyButton value={project.url} label={t.copyUrl} />
</div>
