<script lang="ts">
	import type { FileFilter } from '#lib/backend/ports.js';
	import FormRow from '#lib/components/common/FormRow.svelte';
	import PathInput from '#lib/components/common/PathInput.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { messages as m } from '#lib/messages/index.js';

	interface Props {
		open: boolean;
		title: string;
		description: string;
		label: string;
		/** What the "Sfoglia…" picker selects. */
		mode: 'folder' | 'file' | 'save';
		filters?: FileFilter[];
		placeholder?: string;
		initial?: string;
		confirmLabel: string;
		/** Resolves `true` when the dialog may close. */
		onsubmit: (value: string) => Promise<boolean>;
	}

	let { open = $bindable(), title, description, label, mode, filters, placeholder = '', initial = '', confirmLabel, onsubmit }: Props = $props();
	let value = $state('');
	let busy = $state(false);

	$effect(() => {
		if (open) value = initial;
	});

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		busy = true;
		const ok = await onsubmit(value.trim());
		busy = false;
		if (ok) open = false;
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="max-w-lg">
		<form onsubmit={submit} class="flex flex-col gap-3">
			<Dialog.Header>
				<Dialog.Title>{title}</Dialog.Title>
				<Dialog.Description>{description}</Dialog.Description>
			</Dialog.Header>
			<FormRow label={label} for="path-prompt">
				<PathInput id="path-prompt" bind:value {mode} {filters} {placeholder} title={label} required />
			</FormRow>
			<Dialog.Footer>
				<Button type="button" variant="ghost" size="sm" onclick={() => (open = false)}>{m.common.cancel}</Button>
				<Button type="submit" size="sm" disabled={busy || !value.trim()}>{confirmLabel}</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
