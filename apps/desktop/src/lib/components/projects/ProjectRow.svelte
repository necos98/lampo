<script lang="ts">
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';

	import type { Project } from '#lib/api/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { projects } from '#lib/stores/index.js';

	import ExternalLink from '../common/ExternalLink.svelte';
	import StatusDot from '../common/StatusDot.svelte';
	import ProjectQuickActions from './ProjectQuickActions.svelte';

	let { project }: { project: Project } = $props();
	const tone = $derived(!project.folderExists ? 'warn' : project.active ? 'ok' : 'idle');
	const stateLabel = $derived(!project.folderExists ? m.projects.folderMissing : project.active ? m.projects.active : m.projects.inactive);
</script>

<li class="group relative grid grid-cols-[auto_minmax(0,1.4fr)_minmax(0,1.2fr)_5rem_5rem_auto_auto] items-center gap-x-4 rounded-md border bg-card px-3 py-2.5 transition-colors focus-within:ring-2 focus-within:ring-ring/50 hover:bg-muted/50">
	<StatusDot {tone} label={stateLabel} />
	<div class="min-w-0">
		<a href="/projects/{project.id}" class="block truncate font-medium outline-none after:absolute after:inset-0 after:content-['']">{project.name}</a>
		<p class="flex items-center gap-1 truncate font-mono text-xs text-muted-foreground">
			{#if !project.folderExists}<TriangleAlert class="size-3 shrink-0 text-amber-500" />{/if}
			<span class="truncate">{project.root}</span>
		</p>
	</div>
	<div class="relative z-10 min-w-0 truncate">
		<ExternalLink href={project.url} title={m.projects.open}>{project.url.replace(/^https?:\/\//, '')}</ExternalLink>
	</div>
	<Badge variant="secondary" class="justify-self-start font-mono">PHP {project.environment.phpVersion}</Badge>
	<span class="text-xs text-muted-foreground">
		{project.database ? `MySQL ${project.database.mysqlVersion}` : ''}
	</span>
	<!-- Always in the tab order; shown on hover or when a control of the row has focus. -->
	<ProjectQuickActions {project} class="relative z-10 opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100" />
	<label class="relative z-10 flex items-center gap-2 text-xs text-muted-foreground" title={m.projects.servedHelp}>
		{m.projects.served}
		<Switch
			size="sm"
			checked={project.active}
			disabled={projects.toggling[project.id] || !project.folderExists}
			onCheckedChange={(v) => projects.setActive(project.id, v)}
			aria-label={m.projects.toggle(project.name)}
			title={m.projects.servedHelp}
		/>
	</label>
</li>
