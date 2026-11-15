<script lang="ts">
	import { onMount } from 'svelte';

	import AsyncBoundary from '#lib/components/common/AsyncBoundary.svelte';
	import CopyField from '#lib/components/common/CopyField.svelte';
	import PageHeader from '#lib/components/common/PageHeader.svelte';
	import DoctorPanel from '#lib/components/settings/DoctorPanel.svelte';
	import PresetsPanel from '#lib/components/settings/PresetsPanel.svelte';
	import SettingsForm from '#lib/components/settings/SettingsForm.svelte';
	import ShimsPanel from '#lib/components/settings/ShimsPanel.svelte';
	import UpdatesPanel from '#lib/components/settings/UpdatesPanel.svelte';
	import { Separator } from '#lib/components/ui/separator/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { diagnostics, packages, settings } from '#lib/stores/index.js';

	const info = $derived(diagnostics.info.data);

	onMount(() => {
		void settings.load();
		void diagnostics.info.load();
		if (!packages.catalog.data) void packages.loadLocal();
	});
</script>

<PageHeader title={m.settings.title} />

<div class="max-w-3xl">
	<AsyncBoundary status={settings.value.status} error={settings.value.error} onretry={settings.load} rows={5}>
		{#if settings.value.data}<SettingsForm source={settings.value.data} />{/if}
	</AsyncBoundary>

	<Separator class="my-6" />
	<UpdatesPanel />
	<Separator class="my-6" />
	<ShimsPanel />
	<Separator class="my-6" />
	<PresetsPanel />
	<Separator class="my-6" />
	<DoctorPanel />
	<Separator class="my-6" />

	<h2 class="mb-2 text-sm font-medium">{m.settings.about.title}</h2>
	{#if info}
		<div class="flex flex-col gap-1 pb-6">
			<CopyField label={m.app.version} value={info.version} />
			<CopyField label={m.settings.about.config} value={info.configDir} />
			<CopyField label={m.settings.about.data} value={info.dataDir} />
		</div>
	{/if}
</div>
