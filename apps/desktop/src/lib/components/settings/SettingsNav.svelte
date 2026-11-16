<script lang="ts">
	import Save from '@lucide/svelte/icons/save';
	import Zap from '@lucide/svelte/icons/zap';

	import * as Tabs from '#lib/components/ui/tabs/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { SETTINGS_SECTION_KINDS, sectionsOfKind, type SettingsSectionKind } from '#lib/settings/sections.js';
	import { updater } from '#lib/stores/index.js';

	/** `dirty`: the draft of the saved sections has edits not saved yet. */
	let { dirty }: { dirty: boolean } = $props();

	const t = m.settings;
	const icons = { saved: Save, immediate: Zap } satisfies Record<SettingsSectionKind, typeof Save>;
	const groupId = (kind: SettingsSectionKind) => `settings-kind-${kind}`;
	// bits-ui marks the open tab with data-state, not the data-active the generated Tabs styles expect.
	const triggerClass = 'h-8 w-full flex-none justify-start px-2.5 hover:bg-muted/60 data-[state=active]:bg-muted data-[state=active]:text-foreground';
</script>

<Tabs.List variant="line" class="flex w-48 shrink-0 flex-col items-stretch gap-0.5 p-0" aria-label={t.nav}>
	{#each SETTINGS_SECTION_KINDS as kind (kind)}
		{@const Icon = icons[kind]}
		<!-- Read to screen readers as the description of each tab of the group, not on its own. -->
		<div id={groupId(kind)} aria-hidden="true" class="flex items-center gap-1.5 px-2 pt-3 pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase first:pt-0">
			<Icon class="size-3" />
			{t.kinds[kind].title}
			{#if kind === 'saved' && dirty}
				<span class="ml-auto rounded bg-amber-500/15 px-1 py-px text-[10px] tracking-normal text-amber-600 normal-case dark:text-amber-400">{m.common.unsaved}</span>
			{/if}
		</div>
		{#each sectionsOfKind(kind) as section (section.id)}
			<Tabs.Trigger value={section.id} class={triggerClass} aria-describedby={groupId(kind)}>
				{t.sections[section.id]}
				{#if section.id === 'updates' && updater.available}
					<span class="ml-auto size-2 rounded-full bg-primary" aria-hidden="true"></span>
					<span class="sr-only">{t.updates.dot}</span>
				{/if}
			</Tabs.Trigger>
		{/each}
	{/each}
</Tabs.List>
