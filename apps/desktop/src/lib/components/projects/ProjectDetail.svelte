<script lang="ts">
	import BookmarkPlus from '@lucide/svelte/icons/bookmark-plus';
	import Code from '@lucide/svelte/icons/code';
	import FileUp from '@lucide/svelte/icons/file-up';
	import ListX from '@lucide/svelte/icons/list-x';
	import SquareTerminal from '@lucide/svelte/icons/square-terminal';
	import ScrollText from '@lucide/svelte/icons/scroll-text';
	import { onMount, untrack } from 'svelte';

	import { goto } from '$app/navigation';

	import type { AppError, Project } from '#lib/api/index.js';
	import ConfirmDialog from '#lib/components/common/ConfirmDialog.svelte';
	import ExternalChangeNotice from '#lib/components/common/ExternalChangeNotice.svelte';
	import ExternalLink from '#lib/components/common/ExternalLink.svelte';
	import PageHeader from '#lib/components/common/PageHeader.svelte';
	import SaveBar from '#lib/components/common/SaveBar.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import * as Tabs from '#lib/components/ui/tabs/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { envVarRefusal } from '#lib/projects/envVarRefusal.js';
	import { refusedDirectivesReason } from '#lib/services/errors.js';
	import { projects, unsavedChanges } from '#lib/stores/index.js';
	import { ProjectDraft } from '#lib/stores/projectDraft.svelte.js';

	import WorktreePanel from '../worktrees/WorktreePanel.svelte';
	import PathPromptDialog from './PathPromptDialog.svelte';
	import SavePresetDialog from './SavePresetDialog.svelte';
	import AddressesSection from './sections/AddressesSection.svelte';
	import ApacheSection from './sections/ApacheSection.svelte';
	import DataSection from './sections/DataSection.svelte';
	import EnvVarsSection from './sections/EnvVarsSection.svelte';
	import GeneralSection from './sections/GeneralSection.svelte';
	import PhpSection from './sections/PhpSection.svelte';

	let { project }: { project: Project } = $props();
	const t = m.projects;

	const draft = new ProjectDraft(untrack(() => project));
	let tab = $state('general');
	let saving = $state(false);
	let saveError = $state<AppError | null>(null);
	let relinkOpen = $state(false);
	let exportOpen = $state(false);
	let presetOpen = $state(false);
	let deleteOpen = $state(false);

	// Backend changes (events, another window, the CLI) flow in unless the user is mid-edit; then
	// the notice asks whether to reload them or overwrite them on save.
	const changedElsewhere = $derived(!saving && draft.dirty && draft.isStale(project));
	$effect(() => {
		if (!draft.dirty && draft.isStale(project)) draft.reset(project);
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
		saveError = null;
		draft.reset(project);
	}

	async function save() {
		if (saving || !draft.dirty || !draft.valid) return;
		saving = true;
		saveError = null;
		const result = await projects.update(draft.toRequest());
		saving = false;
		if (result.ok) draft.reset(result.value);
		else {
			saveError = result.error;
			if (result.error.code === 'config-rejected' || refusedDirectivesReason(result.error) !== null) tab = 'apache';
			else if (envVarRefusal(result.error) !== null) tab = 'env';
		}
	}

	async function remove() {
		const result = await projects.remove(project.id);
		if (result.ok) {
			// The project is gone and its edits with it: leaving must not ask to keep them.
			draft.reset(project);
			await goto('/projects');
		}
		return result;
	}

</script>

<PageHeader
	title={project.name}
	subtitle={project.root}
	subtitleTitle={t.openFolder}
	subtitleDisabled={!project.folderExists}
	onsubtitleclick={() => projects.openFolder(project.id)}
>
	{#snippet meta()}
		<ExternalLink href={project.url}>{project.url.replace(/^https?:\/\//, '')}</ExternalLink>
	{/snippet}
	{#snippet actions()}
		<Button variant="ghost" size="sm" disabled={!project.folderExists} onclick={() => projects.openTerminal(project.id)}><SquareTerminal />{t.openTerminal}</Button>
		<Button variant="ghost" size="sm" title={t.openInVsCodeHelp} disabled={!project.folderExists} onclick={() => projects.openInVsCode(project.id)}><Code />{t.openInVsCode}</Button>
		<Button variant="ghost" size="sm" href="/logs?source=error:{project.id}"><ScrollText />{m.nav.logs}</Button>
		<Button variant="ghost" size="sm" onclick={() => (exportOpen = true)}><FileUp />{t.export}</Button>
		<Button variant="ghost" size="sm" onclick={() => (presetOpen = true)}><BookmarkPlus />{t.saveAsPreset}</Button>
		<label class="flex items-center gap-2 text-sm" title={t.servedHelp}>
			{t.served}
			<Switch checked={project.active} disabled={projects.toggling[project.id] || !project.folderExists} onCheckedChange={(v) => projects.setActive(project.id, v)} title={t.servedHelp} />
		</label>
	{/snippet}
</PageHeader>

{#if changedElsewhere}
	<ExternalChangeNotice onreload={discard} onoverwrite={() => draft.rebase(project)} />
{/if}

<Tabs.Root bind:value={tab}>
	<Tabs.List>
		<Tabs.Trigger value="general">{t.tabs.general}</Tabs.Trigger>
		<Tabs.Trigger value="php">{t.tabs.php}</Tabs.Trigger>
		<Tabs.Trigger value="addresses">{t.tabs.addresses}</Tabs.Trigger>
		<Tabs.Trigger value="data">{t.tabs.data}</Tabs.Trigger>
		<Tabs.Trigger value="env">{t.tabs.envVars}</Tabs.Trigger>
		<Tabs.Trigger value="apache">{t.tabs.apache}</Tabs.Trigger>
		<Tabs.Trigger value="worktrees">{t.tabs.worktrees}{project.worktrees.length ? ` (${project.worktrees.length})` : ''}</Tabs.Trigger>
	</Tabs.List>
	<div class="pt-3">
		<Tabs.Content value="general">
			<GeneralSection {draft} {project} onrelink={() => (relinkOpen = true)} />
			<div class="mt-6 border-t pt-4">
				<Button variant="destructive" size="sm" onclick={() => (deleteOpen = true)}><ListX />{t.remove}</Button>
			</div>
		</Tabs.Content>
		<Tabs.Content value="php"><PhpSection {draft} /></Tabs.Content>
		<Tabs.Content value="addresses"><AddressesSection {draft} /></Tabs.Content>
		<Tabs.Content value="data"><DataSection {draft} {project} /></Tabs.Content>
		<Tabs.Content value="env"><EnvVarsSection {draft} rejected={saveError} /></Tabs.Content>
		<Tabs.Content value="apache"><ApacheSection {draft} rejected={saveError} /></Tabs.Content>
		<Tabs.Content value="worktrees"><WorktreePanel projectId={project.id} /></Tabs.Content>
	</div>
</Tabs.Root>

<SaveBar dirty={draft.dirty} valid={draft.valid} {saving} onsave={save} onreset={discard} />

<PathPromptDialog
	bind:open={relinkOpen}
	title={t.relink}
	description={t.relinkHelp}
	label={t.folderPath}
	mode="folder"
	initial={project.root}
	confirmLabel={t.relink}
	onsubmit={async (root) => (await projects.relink({ projectId: project.id, root })).ok}
/>
<PathPromptDialog
	bind:open={exportOpen}
	title={t.export}
	description={t.exportHelp}
	label={t.filePath}
	mode="save"
	filters={[{ name: t.exportFileType, extensions: ['toml'] }]}
	initial="{project.root}\{project.id}.lampo.toml"
	confirmLabel={t.export}
	onsubmit={async (file) => (await projects.export({ projectId: project.id, file })).ok}
/>
<SavePresetDialog bind:open={presetOpen} projectId={project.id} defaultName={project.name} />
<ConfirmDialog bind:open={deleteOpen} title={t.deleteTitle} body={t.deleteBody(project.name)} confirmLabel={t.remove} destructive onconfirm={remove} />
