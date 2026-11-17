<script lang="ts">
	import { untrack } from 'svelte';

	import PathInput from '#lib/components/common/PathInput.svelte';
	import { messages as m } from '#lib/messages/index.js';

	import DefenderExclusionOption from './DefenderExclusionOption.svelte';

	interface Props {
		/** The folder Lampo suggests from the registered projects, `null` when there is none. */
		suggested: string | null;
	}

	let { suggested }: Props = $props();

	const t = m.settings.defender.projects;
	// The suggestion only fills the field: the user may choose any folder (the backend checks it).
	let folder = $state(untrack(() => suggested ?? ''));
	const chosen = $derived(folder.trim());
</script>

<DefenderExclusionOption target="projects" warning paths={chosen ? [chosen] : []} projectsFolder={chosen} confirmBody={t.confirmAdd.body(chosen)}>
	<label for="defender-projects-folder" class="mt-2 block text-xs text-muted-foreground">{t.targets}</label>
	<div class="mt-1">
		<PathInput id="defender-projects-folder" aria-describedby="defender-projects-hint" mode="folder" bind:value={folder} title={t.picker} />
	</div>
	<p id="defender-projects-hint" class="mt-1 text-xs text-muted-foreground">
		{suggested ? t.suggested : t.noSuggestion}
		{t.rules}
	</p>
</DefenderExclusionOption>
