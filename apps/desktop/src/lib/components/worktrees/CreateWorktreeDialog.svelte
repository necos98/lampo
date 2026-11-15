<script lang="ts">
	import type { DatabaseMode } from '#lib/api/index.js';
	import FormRow from '#lib/components/common/FormRow.svelte';
	import SelectField from '#lib/components/common/SelectField.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import PathInput from '#lib/components/common/PathInput.svelte';
	import { messages as m } from '#lib/messages/index.js';
	import { projects, worktrees } from '#lib/stores/index.js';

	interface Props {
		open: boolean;
		projectId: string;
		/** Receives the folder once the task started, so the panel can follow it. */
		onstarted: (path: string) => void;
	}

	let { open = $bindable(), projectId, onstarted }: Props = $props();
	const t = m.worktrees;

	let path = $state('');
	let mode = $state<DatabaseMode['mode']>('fresh');
	let source = $state('');
	let busy = $state(false);

	const modeOptions = (['shared', 'fresh', 'clone'] as const).map((value) => ({ value, label: t.modes[value] }));
	const sourceOptions = $derived((projects.list.data ?? []).filter((p) => p.database).map((p) => ({ value: p.id, label: p.name })));
	const effectiveSource = $derived(source || projectId);

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		busy = true;
		const database: DatabaseMode = mode === 'clone' ? { mode: 'clone', source: effectiveSource } : { mode };
		const folder = path.trim();
		const result = await worktrees.create(projectId, folder, database);
		busy = false;
		if (result.ok) {
			onstarted(folder);
			open = false;
			path = '';
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="max-w-lg">
		<form onsubmit={submit} class="flex flex-col gap-3">
			<Dialog.Header>
				<Dialog.Title>{t.createTitle}</Dialog.Title>
				<Dialog.Description>{t.createHelp}</Dialog.Description>
			</Dialog.Header>
			<FormRow label={t.createPath} for="wt-path">
				<PathInput id="wt-path" mode="folder" bind:value={path} title={t.createPath} placeholder="C:\Dev\.worktrees\feature-x" required />
			</FormRow>
			<FormRow label={t.createDb}>
				<SelectField value={mode} options={modeOptions} class="w-full" onchange={(v) => (mode = v as DatabaseMode['mode'])} />
			</FormRow>
			{#if mode === 'clone'}
				<FormRow label={t.cloneSource}>
					<SelectField value={effectiveSource} options={sourceOptions} class="w-full" onchange={(v) => (source = v)} />
				</FormRow>
			{/if}
			<Dialog.Footer>
				<Button type="button" variant="ghost" size="sm" onclick={() => (open = false)}>{m.common.cancel}</Button>
				<Button type="submit" size="sm" disabled={busy || !path.trim()}>{t.create}</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
