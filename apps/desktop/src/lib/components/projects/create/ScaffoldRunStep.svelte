<script lang="ts">
	import { tick } from 'svelte';

	import ErrorNotice from '#lib/components/common/ErrorNotice.svelte';
	import TaskProgress from '#lib/components/common/TaskProgress.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { describeError } from '#lib/services/errors.js';
	import type { CreateProjectFlow } from '#lib/stores/createProject.svelte.js';
	import type { TaskState } from '#lib/stores/tasks.svelte.js';

	/** The setup task of a new folder: progress, live log and its outcome. */
	let { flow, task }: { flow: CreateProjectFlow; task: TaskState } = $props();
	const t = m.create;

	// Log box: follow the output while it streams.
	let logBox = $state<HTMLPreElement | null>(null);
	$effect(() => {
		void task.lines.length;
		void tick().then(() => logBox?.scrollTo({ top: logBox.scrollHeight }));
	});
</script>

<Card.Root size="sm">
	<Card.Header>
		<Card.Title>{t.stepRun}</Card.Title>
		<Card.Description>{flow.source === 'git' ? t.running.git : t.running.laravel}</Card.Description>
	</Card.Header>
	<Card.Content class="flex flex-col gap-3">
		{#if flow.phase === 'running'}
			<TaskProgress progress={task.progress} oncancel={() => flow.cancelScaffold()} />
		{/if}
		<div>
			<p class="mb-1 text-xs font-medium text-muted-foreground">{t.log}</p>
			<pre
				bind:this={logBox}
				class="h-64 overflow-auto rounded-md border bg-muted/40 p-2 font-mono text-xs leading-relaxed whitespace-pre-wrap break-all"
				aria-live="polite">{task.lines.length ? task.lines.join('\n') : t.logWaiting}</pre>
		</div>
		{#if flow.phase === 'failed' && flow.scaffoldError}
			<!-- Retry lives in the footer, next to Modifica: one block, one button. -->
			<ErrorNotice error={flow.scaffoldError} title={t.failed} description={describeError(flow.scaffoldError)} />
		{/if}
		{#if flow.phase === 'done'}
			<p class="text-sm text-emerald-700 dark:text-emerald-400">{t.done(flow.folder)}</p>
		{/if}
	</Card.Content>
	{#if flow.phase === 'running'}
		<Card.Footer class="justify-end">
			<Button type="button" size="sm" variant="outline" onclick={() => flow.cancelScaffold()}>{m.common.cancel}</Button>
		</Card.Footer>
	{:else if flow.phase === 'failed'}
		<Card.Footer class="justify-end gap-2">
			<Button type="button" size="sm" variant="ghost" onclick={() => flow.editScaffold()}>{t.edit}</Button>
			<Button type="button" size="sm" onclick={() => flow.startScaffold()}>{m.common.retry}</Button>
		</Card.Footer>
	{/if}
</Card.Root>
