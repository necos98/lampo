<script lang="ts">
	import Save from '@lucide/svelte/icons/save';
	import Zap from '@lucide/svelte/icons/zap';
	import type { Snippet } from 'svelte';

	import * as Tabs from '#lib/components/ui/tabs/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { settingsSection, type SettingsSectionId } from '#lib/settings/sections.js';

	let { id, children }: { id: SettingsSectionId; children: Snippet } = $props();

	const kind = $derived(settingsSection(id).kind);
</script>

<!-- Inactive panels stay mounted (only hidden), so an edited draft survives a change of tab. -->
<Tabs.Content value={id} class="min-w-0">
	<p class="mb-4 flex items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-xs {kind === 'saved' ? 'border-primary/30 bg-primary/5' : 'bg-muted/50 text-muted-foreground'}">
		{#if kind === 'saved'}<Save class="size-3.5 shrink-0" />{:else}<Zap class="size-3.5 shrink-0" />{/if}
		{m.settings.kinds[kind].hint}
	</p>
	{@render children()}
</Tabs.Content>
