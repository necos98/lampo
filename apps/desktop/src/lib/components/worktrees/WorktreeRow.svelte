<script lang="ts">
	import GitBranch from '@lucide/svelte/icons/git-branch';
	import Trash2 from '@lucide/svelte/icons/trash-2';

	import type { WorktreeEnvironment } from '#lib/api/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { formatDateTime } from '#lib/services/format.js';
	import { worktrees } from '#lib/stores/index.js';

	import ExternalLink from '../common/ExternalLink.svelte';

	let { worktree, onremove }: { worktree: WorktreeEnvironment; onremove: (w: WorktreeEnvironment) => void } = $props();
	const t = m.worktrees;
</script>

<li class="grid grid-cols-[minmax(0,1.2fr)_minmax(0,1.6fr)_7rem_auto] items-center gap-x-4 rounded-md border bg-card px-3 py-2">
	<div class="min-w-0">
		<p class="flex items-center gap-1.5 truncate font-medium">
			<GitBranch class="size-3.5 shrink-0 text-muted-foreground" />
			<span class="truncate">{worktree.branch ?? t.noBranch}</span>
			{#if worktree.stale}<Badge variant="destructive">{t.stale}</Badge>{/if}
		</p>
		<p class="truncate text-xs text-muted-foreground" title={formatDateTime(worktree.createdAtUnixMs)}>{worktree.site}</p>
	</div>
	<div class="min-w-0">
		<ExternalLink href={worktree.url}>{worktree.url.replace(/^https?:\/\//, '')}</ExternalLink>
		<p class="truncate font-mono text-xs text-muted-foreground" title={worktree.root}>{worktree.root}</p>
	</div>
	<Badge variant="secondary" title={t.modes[worktree.databaseMode.mode]}>{t.modeShort[worktree.databaseMode.mode]}</Badge>
	<Button variant="ghost" size="icon-xs" onclick={() => onremove(worktree)} disabled={worktrees.removing[worktree.site]} aria-label="{t.remove} {worktree.site}"><Trash2 /></Button>
</li>
