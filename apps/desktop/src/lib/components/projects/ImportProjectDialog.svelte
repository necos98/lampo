<script lang="ts">
	import { goto } from '$app/navigation';

	import FormRow from '#lib/components/common/FormRow.svelte';
	import PathInput from '#lib/components/common/PathInput.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { projects } from '#lib/stores/index.js';

	let { open = $bindable() }: { open: boolean } = $props();
	let file = $state('');
	let root = $state('');
	let busy = $state(false);

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		busy = true;
		const result = await projects.import({ file: file.trim(), root: root.trim() || null });
		busy = false;
		if (result.ok) {
			open = false;
			file = root = '';
			await goto(`/projects/${result.value.id}`);
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="max-w-lg">
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
	</Dialog.Content>
</Dialog.Root>
