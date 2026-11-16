<script lang="ts">
	import { onMount } from 'svelte';

	import AsyncBoundary from '#lib/components/common/AsyncBoundary.svelte';
	import PageHeader from '#lib/components/common/PageHeader.svelte';
	import AboutPanel from '#lib/components/settings/AboutPanel.svelte';
	import ClaudeCodePanel from '#lib/components/settings/ClaudeCodePanel.svelte';
	import DoctorPanel from '#lib/components/settings/DoctorPanel.svelte';
	import PresetsPanel from '#lib/components/settings/PresetsPanel.svelte';
	import SettingsForm from '#lib/components/settings/SettingsForm.svelte';
	import SettingsNav from '#lib/components/settings/SettingsNav.svelte';
	import SettingsPanel from '#lib/components/settings/SettingsPanel.svelte';
	import ShimsPanel from '#lib/components/settings/ShimsPanel.svelte';
	import UpdatesPanel from '#lib/components/settings/UpdatesPanel.svelte';
	import { Separator } from '#lib/components/ui/separator/index.js';
	import * as Tabs from '#lib/components/ui/tabs/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { DEFAULT_SETTINGS_SECTION, isSavedSection } from '#lib/settings/sections.js';
	import { settings } from '#lib/stores/index.js';

	let tab = $state<string>(DEFAULT_SETTINGS_SECTION);
	let dirty = $state(false);
	const savedTab = $derived(isSavedSection(tab));

	onMount(() => void settings.load());
</script>

<PageHeader title={m.settings.title} />

<Tabs.Root bind:value={tab} orientation="vertical" class="max-w-4xl items-start gap-6">
	<SettingsNav {dirty} />
	<div class="min-w-0 flex-1">
		<!-- Once loaded the form stays mounted on every tab, so its draft survives a change of tab. -->
		{#if settings.value.data || savedTab}
			<AsyncBoundary status={settings.value.status} error={settings.value.error} onretry={settings.load} rows={5}>
				{#if settings.value.data}<SettingsForm source={settings.value.data} shown={savedTab} bind:dirty />{/if}
			</AsyncBoundary>
		{/if}

		<SettingsPanel id="updates"><UpdatesPanel /></SettingsPanel>
		<SettingsPanel id="tools">
			<ShimsPanel />
			<Separator class="my-6" />
			<ClaudeCodePanel />
		</SettingsPanel>
		<SettingsPanel id="presets"><PresetsPanel /></SettingsPanel>
		<SettingsPanel id="doctor"><DoctorPanel /></SettingsPanel>
		<SettingsPanel id="about"><AboutPanel /></SettingsPanel>
	</div>
</Tabs.Root>
