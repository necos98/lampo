<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { SHORTCUT_HELP } from '#lib/services/navigation.js';

	let { open = $bindable(false) }: { open: boolean } = $props();
	const t = m.shortcuts;
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="max-w-md">
		<Dialog.Header>
			<Dialog.Title>{t.title}</Dialog.Title>
			<Dialog.Description>{t.description}</Dialog.Description>
		</Dialog.Header>
		<dl class="grid grid-cols-[auto_1fr] items-center gap-x-4 gap-y-2">
			{#each SHORTCUT_HELP as entry (entry.id)}
				<dt class="flex items-center gap-1">
					{#each entry.keys as key, i (i)}
						{#if i > 0}<span class="text-xs text-muted-foreground">+</span>{/if}
						<kbd class="rounded border bg-muted px-1.5 py-0.5 font-mono text-xs">{key}</kbd>
					{/each}
				</dt>
				<dd class="text-sm">{t.help[entry.id]}</dd>
			{/each}
		</dl>
	</Dialog.Content>
</Dialog.Root>
