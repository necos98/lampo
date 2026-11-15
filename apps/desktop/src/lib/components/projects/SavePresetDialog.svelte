<script lang="ts">
	import FormRow from '#lib/components/common/FormRow.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { presets } from '#lib/stores/index.js';

	let { open = $bindable(), projectId, defaultName }: { open: boolean; projectId: string; defaultName: string } = $props();
	let name = $state('');
	let description = $state('');
	let busy = $state(false);

	$effect(() => {
		if (open) {
			name = defaultName;
			description = '';
		}
	});

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		busy = true;
		const result = await presets.save(projectId, name.trim(), description.trim());
		busy = false;
		if (result.ok) open = false;
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="max-w-lg">
		<form onsubmit={submit} class="flex flex-col gap-3">
			<Dialog.Header>
				<Dialog.Title>{m.projects.saveAsPreset}</Dialog.Title>
				<Dialog.Description>{m.projects.saveAsPresetHelp}</Dialog.Description>
			</Dialog.Header>
			<FormRow label={m.projects.presetName} for="preset-name"><Input id="preset-name" class="h-8" bind:value={name} required /></FormRow>
			<FormRow label={m.projects.presetDescription} for="preset-desc"><Input id="preset-desc" class="h-8" bind:value={description} /></FormRow>
			<Dialog.Footer>
				<Button type="button" variant="ghost" size="sm" onclick={() => (open = false)}>{m.common.cancel}</Button>
				<Button type="submit" size="sm" disabled={busy || !name.trim()}>{m.common.save}</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
