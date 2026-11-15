<script lang="ts">
	import X from '@lucide/svelte/icons/x';

	import type { TaskProgressPayload } from '#lib/api/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Progress } from '#lib/components/ui/progress/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { formatBytes } from '#lib/services/format.js';

	interface Props {
		progress: TaskProgressPayload | null;
		oncancel?: () => void;
	}

	let { progress, oncancel }: Props = $props();
	// `running` counts steps, not bytes, and names the program: an indeterminate bar fits better.
	const running = $derived(progress?.stage === 'running');
	const determinate = $derived(!running && progress?.total != null && progress.total > 0);
	const stage = $derived(progress ? m.packages.stages[progress.stage] : m.packages.stages.resolving);
	const detail = $derived(
		running && progress?.subject
			? `${stage} · ${progress.subject}`
			: determinate && progress
				? `${stage} · ${formatBytes(progress.done)} / ${formatBytes(progress.total)}`
				: stage
	);
</script>

<div class="flex items-center gap-3">
	<div class="min-w-0 flex-1">
		{#if determinate && progress}
			<Progress value={progress.done} max={progress.total ?? 1} aria-label={stage} />
		{:else}
			<div class="relative h-1.5 w-full overflow-hidden rounded-full bg-muted" role="progressbar" aria-label={stage}>
				<div class="absolute inset-y-0 w-1/3 animate-[indeterminate_1.2s_ease-in-out_infinite] rounded-full bg-primary"></div>
			</div>
		{/if}
		<p class="mt-1 truncate text-xs text-muted-foreground">{detail}</p>
	</div>
	{#if oncancel}
		<Button variant="ghost" size="icon-xs" onclick={oncancel} aria-label={m.common.cancel}><X /></Button>
	{/if}
</div>

<style>
	@keyframes indeterminate {
		0% {
			left: -33%;
		}
		100% {
			left: 100%;
		}
	}
</style>
