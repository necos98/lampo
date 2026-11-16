<script lang="ts">
	import { onMount, untrack } from 'svelte';

	import type { Settings } from '#lib/api/index.js';
	import ExternalChangeNotice from '#lib/components/common/ExternalChangeNotice.svelte';
	import SaveBar from '#lib/components/common/SaveBar.svelte';
	import { settings, unsavedChanges } from '#lib/stores/index.js';
	import { SettingsDraft } from '#lib/stores/settingsDraft.svelte.js';

	import SettingsPanel from './SettingsPanel.svelte';
	import GeneralSection from './sections/GeneralSection.svelte';
	import PortsSection from './sections/PortsSection.svelte';

	interface Props {
		source: Settings;
		/** One of this form's sections is the open tab: the save bar and the notices show. */
		shown: boolean;
		/** Out: the draft has edits not saved yet (the side index marks them). */
		dirty?: boolean;
	}

	let { source, shown, dirty = $bindable(false) }: Props = $props();

	// One draft for every "saved" section: a single Save applies the edits of all of them.
	const draft = new SettingsDraft(untrack(() => source));

	$effect(() => {
		dirty = draft.dirty;
	});

	// Backend changes flow in unless the user is mid-edit; then the notice asks what to keep.
	const changedElsewhere = $derived(!settings.saving && draft.dirty && draft.isStale(source));
	$effect(() => {
		if (!draft.dirty && draft.isStale(source)) draft.reset(source);
	});

	onMount(() =>
		unsavedChanges.register({
			get dirty() {
				return draft.dirty;
			},
			save,
			discard
		})
	);

	function discard() {
		draft.reset(source);
	}

	async function save() {
		if (settings.saving || !draft.dirty || !draft.valid) return;
		const result = await settings.save(draft.toSettings());
		if (result.ok) draft.reset(result.value);
	}
</script>

{#if shown && changedElsewhere}
	<ExternalChangeNotice onreload={discard} onoverwrite={() => draft.rebase(source)} />
{/if}

<SettingsPanel id="general"><GeneralSection {draft} /></SettingsPanel>
<SettingsPanel id="ports"><PortsSection {draft} /></SettingsPanel>

{#if shown}
	<SaveBar dirty={draft.dirty} valid={draft.valid} saving={settings.saving} onsave={save} onreset={discard} />
{/if}
