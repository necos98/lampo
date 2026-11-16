<script lang="ts">
	import FolderOpen from '@lucide/svelte/icons/folder-open';
	import GitBranch from '@lucide/svelte/icons/git-branch';
	import Sparkles from '@lucide/svelte/icons/sparkles';

	import { messages as m } from '#lib/messages/index.js';
	import type { CreateProjectFlow, ProjectSource } from '#lib/stores/createProject.svelte.js';

	let { flow }: { flow: CreateProjectFlow } = $props();
	const t = m.create;

	const sourceCards: { id: ProjectSource; icon: typeof FolderOpen }[] = [
		{ id: 'existing', icon: FolderOpen },
		{ id: 'git', icon: GitBranch },
		{ id: 'laravel', icon: Sparkles }
	];
</script>

<div role="radiogroup" aria-label={t.source} class="grid grid-cols-1 gap-2 sm:grid-cols-3">
	{#each sourceCards as card (card.id)}
		{@const selected = flow.source === card.id}
		<button
			type="button"
			role="radio"
			aria-checked={selected}
			disabled={flow.phase === 'running' && !selected}
			onclick={() => flow.setSource(card.id)}
			class="flex flex-col gap-1 rounded-lg border p-3 text-left transition-colors disabled:opacity-50 {selected ? 'border-primary bg-primary/5 ring-1 ring-primary' : 'hover:bg-muted/50'}"
		>
			<span class="flex items-center gap-2 text-sm font-medium"><card.icon class="size-4" />{t.sources[card.id].title}</span>
			<span class="text-xs text-muted-foreground">{t.sources[card.id].description}</span>
		</button>
	{/each}
</div>
