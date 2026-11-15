<script lang="ts">
	import { goto } from '$app/navigation';

	import type { ImportProjectResult } from '#lib/api/index.js';
	import FormRow from '#lib/components/common/FormRow.svelte';
	import PathInput from '#lib/components/common/PathInput.svelte';
	import ImportDroppedNotice from '#lib/components/projects/ImportDroppedNotice.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { projects } from '#lib/stores/index.js';

	let { open = $bindable() }: { open: boolean } = $props();
	let file = $state('');
	let root = $state('');
	let busy = $state(false);
	/** Set after an import that left settings out: the dialog shows them before moving on. */
	let review = $state<ImportProjectResult | null>(null);

	function droppedAnything(r: ImportProjectResult): boolean {
		return r.droppedApacheDirectives !== null || r.droppedPhpIniKeys.length > 0;
	}

	async function openProject(id: string) {
		open = false;
		review = null;
		file = root = '';
		await goto(`/projects/${id}`);
	}

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		busy = true;
		const result = await projects.import({ file: file.trim(), root: root.trim() || null });
		busy = false;
		if (!result.ok) return;
		if (droppedAnything(result.value)) {
			review = result.value;
		} else {
			await openProject(result.value.id);
		}
	}

	$effect(() => {
		if (!open && review) void openProject(review.id);
	});
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="max-w-lg">
		{#if review}
			<div class="flex flex-col gap-3">
				<Dialog.Header>
					<Dialog.Title>{m.projects.imported}</Dialog.Title>
				</Dialog.Header>
				<ImportDroppedNotice result={review} />
				<Dialog.Footer>
					<Button size="sm" onclick={() => review && openProject(review.id)}>{m.projects.openImported}</Button>
				</Dialog.Footer>
			</div>
		{:else}
			<form onsubmit={submit} class="flex flex-col gap-3">
				<Dialog.Header>
					<Dialog.Title>{m.projects.import}</Dialog.Title>
					<Dialog.Description>{m.projects.importHelp}</Dialog.Description>
				</Dialog.Header>
				<FormRow label={m.projects.importFile} for="import-file">
					<PathInput
						id="import-file"
						mode="file"
						bind:value={file}
						title={m.projects.importFile}
						filters={[{ name: m.projects.exportFileType, extensions: ['toml'] }]}
						placeholder="C:\Dev\shop.lampo.toml"
						required
					/>
				</FormRow>
				<FormRow label={m.projects.importRoot} for="import-root">
					<PathInput id="import-root" mode="folder" bind:value={root} title={m.projects.importRoot} placeholder="C:\Dev\shop" />
				</FormRow>
				<Dialog.Footer>
					<Button type="button" variant="ghost" size="sm" onclick={() => (open = false)}>{m.common.cancel}</Button>
					<Button type="submit" size="sm" disabled={busy || !file.trim()}>{m.projects.import}</Button>
				</Dialog.Footer>
			</form>
		{/if}
	</Dialog.Content>
</Dialog.Root>
