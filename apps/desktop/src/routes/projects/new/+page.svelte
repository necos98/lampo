<script lang="ts">
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import Download from '@lucide/svelte/icons/download';
	import FolderOpen from '@lucide/svelte/icons/folder-open';
	import FolderSearch from '@lucide/svelte/icons/folder-search';
	import GitBranch from '@lucide/svelte/icons/git-branch';
	import Play from '@lucide/svelte/icons/play';
	import Sparkles from '@lucide/svelte/icons/sparkles';
	import { onMount, tick } from 'svelte';

	import type { Addressing, StarterKit, TestingFramework } from '#lib/api/index.js';

	import { goto } from '$app/navigation';

	import ErrorNotice from '#lib/components/common/ErrorNotice.svelte';
	import FormRow from '#lib/components/common/FormRow.svelte';
	import PageHeader from '#lib/components/common/PageHeader.svelte';
	import SelectField from '#lib/components/common/SelectField.svelte';
	import PathInput from '#lib/components/common/PathInput.svelte';
	import TaskProgress from '#lib/components/common/TaskProgress.svelte';
	import PhpVersionSelect from '#lib/components/projects/PhpVersionSelect.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { describeError } from '#lib/services/errors.js';
	import { createProjectFlow, packages, presets, tasks } from '#lib/stores/index.js';
	import { LARAVEL_MIN_PHP, compareMinor, type ProjectSource } from '#lib/stores/createProject.svelte.js';

	const flow = createProjectFlow();
	const t = m.create;

	onMount(() => {
		// A clone or Laravel setup started before leaving this page continues where it got to.
		flow.resume();
		if (!presets.list.data) void presets.load();
		if (!packages.catalog.data) void packages.loadLocal();
		return () => flow.dispose();
	});

	const presetOptions = $derived((presets.list.data ?? []).map((p) => ({ value: p.id, label: presets.label(p).name })));
	const addressingOptions = [
		{ value: 'no-elevation', label: m.create.addressingNoElevation },
		{ value: 'standard', label: m.create.addressingStandard }
	];
	const sourceCards: { id: ProjectSource; icon: typeof FolderOpen }[] = [
		{ id: 'existing', icon: FolderOpen },
		{ id: 'git', icon: GitBranch },
		{ id: 'laravel', icon: Sparkles }
	];
	const starterKitOptions = (Object.keys(t.starterKits) as StarterKit[]).map((k) => ({ value: k, label: t.starterKits[k] }));
	const testingOptions = (Object.keys(t.testingFrameworks) as TestingFramework[]).map((k) => ({ value: k, label: t.testingFrameworks[k] }));

	// Laravel: newest installed PHP the installer supports, until the user picks one.
	const newestLaravelPhp = $derived(
		packages.phpOptions
			.filter((o) => o.installed && compareMinor(o.series, LARAVEL_MIN_PHP) >= 0)
			.map((o) => o.series)
			.sort(compareMinor)
			.at(-1) ?? null
	);
	$effect(() => {
		if (flow.source === 'laravel' && flow.laravelPhp === null && newestLaravelPhp) flow.laravelPhp = newestLaravelPhp;
	});

	// Composer is needed by the Laravel Installer: offer to install it right here.
	const composerRows = $derived(packages.rows.filter((r) => r.kind === 'composer').sort((a, b) => compareMinor(a.series, b.series)));
	const composerInstalled = $derived(composerRows.some((r) => r.installed !== null));
	const composerRow = $derived(composerRows.at(-1) ?? null);
	const composerTask = $derived(composerRow ? tasks.forKey(packages.taskKey('composer', composerRow.series, null)) : null);
	const composerInstalling = $derived(composerTask !== null && composerTask.outcome === null);
	/** Also true before the task is tracked: blocks a second click. */
	const composerBusy = $derived(composerRow !== null && packages.isInstalling(composerRow));

	const task = $derived(flow.task);
	const locked = $derived(flow.phase === 'running' || flow.phase === 'done');
	const folderMessages = { empty: t.folderEmpty, invalid: t.folderInvalid, 'laravel-name': t.laravelNameInvalid } as const;
	// Only once something was typed: an untouched form shows no error.
	const folderMessage = $derived(flow.folderProblem && (flow.folderName.trim() || flow.parentDir.trim()) ? folderMessages[flow.folderProblem] : null);
	const scaffoldErrorText = $derived(flow.scaffoldError ? describeError(flow.scaffoldError) : null);

	// Log box: follow the output while it streams.
	let logBox = $state<HTMLPreElement | null>(null);
	$effect(() => {
		void task?.lines.length;
		void tick().then(() => logBox?.scrollTo({ top: logBox.scrollHeight }));
	});

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (flow.source !== 'existing' && !flow.detection) {
			if (flow.phase === 'form' || flow.phase === 'failed') await flow.startScaffold();
			return;
		}
		if (!flow.detection) return void flow.detect();
		const project = await flow.create();
		if (project) await goto(`/projects/${project.id}`);
	}
</script>

<PageHeader title={m.create.title}>
	{#snippet actions()}
		<Button variant="ghost" size="sm" href="/projects"><ArrowLeft />{m.projects.backToList}</Button>
	{/snippet}
</PageHeader>

<form onsubmit={submit} class="flex max-w-2xl flex-col gap-4">
	<div role="radiogroup" aria-label={t.source} class="grid grid-cols-1 gap-2 sm:grid-cols-3">
		{#each sourceCards as card (card.id)}
			{@const selected = flow.source === card.id}
			<button
				type="button"
				role="radio"
				aria-checked={selected}
				disabled={flow.phase === 'running' && !selected}
				onclick={() => flow.setSource(card.id)}
				class="flex flex-col gap-1 rounded-lg border p-3 text-left transition-colors disabled:opacity-50 {selected ? 'border-primary bg-primary/5 ring-1 ring-primary' : 'hover:bg-muted/50'}"
			>
				<span class="flex items-center gap-2 text-sm font-medium"><card.icon class="size-4" />{t.sources[card.id].title}</span>
				<span class="text-xs text-muted-foreground">{t.sources[card.id].description}</span>
			</button>
		{/each}
	</div>

	{#if flow.source === 'existing'}
		<Card.Root size="sm">
			<Card.Header><Card.Title>{m.create.stepFolder}</Card.Title></Card.Header>
			<Card.Content class="flex flex-col gap-2">
				<FormRow label={m.create.folderLabel} for="folder" help={m.create.folderHelp}>
					<div class="flex gap-2">
						<div class="flex-1">
							<PathInput
								id="folder"
								mode="folder"
								title={m.create.folderLabel}
								placeholder={m.create.folderPlaceholder}
								bind:value={flow.folder}
								oninput={() => flow.folderEdited()}
								onblur={() => flow.folder.trim() && !flow.detection && void flow.detect()}
								onpicked={() => void flow.detect()}
								autofocus
							/>
						</div>
						<Button type="button" size="sm" variant="secondary" onclick={() => flow.detect()} disabled={!flow.folder.trim() || flow.detecting}>
							<FolderSearch />{flow.detecting ? m.create.detecting : m.create.detect}
						</Button>
					</div>
				</FormRow>
				{#if flow.detectError}<ErrorNotice error={flow.detectError} title={m.common.loadFailed} />{/if}
			</Card.Content>
		</Card.Root>
	{:else}
		<Card.Root size="sm">
			<Card.Header>
				<Card.Title>{t.stepSource}</Card.Title>
				{#if flow.source === 'laravel'}<Card.Description>{t.installerNote}</Card.Description>{/if}
			</Card.Header>
			<Card.Content class="flex flex-col gap-1">
				<fieldset disabled={locked} class="contents">
					{#if flow.source === 'git'}
						<FormRow label={t.repoUrl} for="repo-url" help={t.repoUrlHelp}>
							<Input id="repo-url" class="h-8" placeholder={t.repoUrlPlaceholder} bind:value={flow.repoUrl} oninput={() => flow.repoUrlEdited()} autocomplete="off" spellcheck={false} />
						</FormRow>
						<FormRow label={t.branch} for="branch">
							<Input id="branch" class="h-8 max-w-xs" placeholder={t.branchPlaceholder} bind:value={flow.branch} autocomplete="off" spellcheck={false} />
						</FormRow>
					{:else}
						<FormRow label={t.laravelName} for="laravel-name" help={t.laravelNameHelp}>
							<Input id="laravel-name" class="h-8 max-w-xs" placeholder="mio-progetto" bind:value={flow.folderName} oninput={() => flow.folderNameEdited()} autocomplete="off" spellcheck={false} />
						</FormRow>
					{/if}

					<FormRow label={t.parentDir} for="parent-dir" help={t.parentDirHelp}>
						<PathInput id="parent-dir" mode="folder" title={t.parentDir} placeholder={t.parentDirPlaceholder} bind:value={flow.parentDir} />
					</FormRow>

					{#if flow.source === 'git'}
						<FormRow label={t.folderName} for="folder-name" help={t.folderNameHelp}>
							<Input id="folder-name" class="h-8 max-w-xs" bind:value={flow.folderName} oninput={() => flow.folderNameEdited()} autocomplete="off" spellcheck={false} />
						</FormRow>
					{:else}
						<FormRow label={t.laravelPhp} help={t.laravelPhpHelp}>
							<PhpVersionSelect bind:value={flow.laravelPhp} />
							{#if !flow.laravelPhpSupported}<p class="mt-1 text-xs text-destructive">{t.laravelPhpTooOld}</p>{/if}
						</FormRow>
						<FormRow label={t.starterKit}>
							<SelectField value={flow.starterKit} options={starterKitOptions} class="w-48" onchange={(v) => (flow.starterKit = v as StarterKit)} />
						</FormRow>
						<FormRow label={t.testing}>
							<SelectField value={flow.testing} options={testingOptions} class="w-48" onchange={(v) => (flow.testing = v as TestingFramework)} />
						</FormRow>
						<FormRow label={t.initGit} for="init-git" help={t.initGitHelp}>
							<Switch id="init-git" bind:checked={flow.initGit} />
						</FormRow>
						<FormRow label={t.npm} for="npm" help={t.npmHelp}>
							<Switch id="npm" bind:checked={flow.npm} />
						</FormRow>
						{#if packages.installed.data && !composerInstalled}
							<div class="flex items-center gap-3 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm">
								<span class="flex-1">{t.composerMissing}</span>
								{#if composerInstalling && composerTask}
									<div class="w-48"><TaskProgress progress={composerTask.progress} oncancel={() => tasks.cancel(composerTask.taskId)} /></div>
								{:else if composerRow}
									<Button type="button" size="xs" variant="outline" disabled={composerBusy} onclick={() => packages.install(composerRow)}><Download />{t.installComposer}</Button>
								{/if}
							</div>
						{/if}
					{/if}
				</fieldset>

				{#if folderMessage}
					<p class="text-xs text-destructive" role="alert">{folderMessage}</p>
				{:else if flow.destination}
					<p class="text-xs text-muted-foreground">{t.destination(flow.destination)}</p>
				{/if}
				{#if flow.phase === 'form' && flow.scaffoldError}
					<p class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive" role="alert">
						{scaffoldErrorText}<span class="mt-1 block font-mono text-xs opacity-80">{flow.scaffoldError.message}</span>
					</p>
				{/if}
				{#if flow.phase === 'form' && flow.cancelled}<p class="text-xs text-muted-foreground">{t.cancelledNotice}</p>{/if}
			</Card.Content>
			{#if flow.phase === 'form'}
				<Card.Footer class="justify-end">
					<Button type="submit" size="sm" disabled={!flow.canStart}><Play />{flow.starting ? t.starting : t.start}</Button>
				</Card.Footer>
			{/if}
		</Card.Root>

		{#if flow.phase !== 'form' && task}
			<Card.Root size="sm">
				<Card.Header>
					<Card.Title>{t.stepRun}</Card.Title>
					<Card.Description>{flow.source === 'git' ? t.running.git : t.running.laravel}</Card.Description>
				</Card.Header>
				<Card.Content class="flex flex-col gap-3">
					{#if flow.phase === 'running'}
						<TaskProgress progress={task.progress} oncancel={() => flow.cancelScaffold()} />
					{/if}
					<div>
						<p class="mb-1 text-xs font-medium text-muted-foreground">{t.log}</p>
						<pre
							bind:this={logBox}
							class="h-64 overflow-auto rounded-md border bg-muted/40 p-2 font-mono text-xs leading-relaxed whitespace-pre-wrap break-all"
							aria-live="polite">{task.lines.length ? task.lines.join('\n') : t.logWaiting}</pre>
					</div>
					{#if flow.phase === 'failed' && flow.scaffoldError}
						<!-- Retry lives in the footer, next to Modifica: one block, one button. -->
						<ErrorNotice error={flow.scaffoldError} title={t.failed} description={scaffoldErrorText} />
					{/if}
					{#if flow.phase === 'done'}
						<p class="text-sm text-emerald-700 dark:text-emerald-400">{t.done(flow.folder)}</p>
					{/if}
				</Card.Content>
				{#if flow.phase === 'running'}
					<Card.Footer class="justify-end">
						<Button type="button" size="sm" variant="outline" onclick={() => flow.cancelScaffold()}>{m.common.cancel}</Button>
					</Card.Footer>
				{:else if flow.phase === 'failed'}
					<Card.Footer class="justify-end gap-2">
						<Button type="button" size="sm" variant="ghost" onclick={() => flow.editScaffold()}>{t.edit}</Button>
						<Button type="button" size="sm" onclick={() => flow.startScaffold()}>{m.common.retry}</Button>
					</Card.Footer>
				{/if}
			</Card.Root>
		{/if}

		{#if flow.phase === 'done' && flow.detectError}<ErrorNotice error={flow.detectError} title={m.common.loadFailed} onretry={() => flow.detect()} />{/if}
	{/if}

	{#if flow.detection}
		{@const d = flow.detection}
		<Card.Root size="sm">
			<Card.Header>
				<Card.Title>{flow.source === 'existing' ? m.create.stepConfirm : t.stepConfirmNew}</Card.Title>
				<Card.Description>
					{#if d.projectType}{m.create.detected}: <Badge variant="secondary">{presets.list.data?.find((p) => p.id === d.presetId) ? presets.label(presets.list.data.find((p) => p.id === d.presetId)!).name : d.projectType}</Badge>
					{:else}{m.create.notDetected}{/if}
				</Card.Description>
			</Card.Header>
			<Card.Content class="flex flex-col gap-1">
				{#if d.alreadyRegistered}
					<div class="mb-2 flex items-center gap-3 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm">
						<span class="flex-1">{m.create.alreadyRegistered(d.alreadyRegistered)}</span>
						<Button size="xs" variant="outline" href="/projects/{d.alreadyRegistered}">{m.create.openExisting}</Button>
					</div>
				{/if}
				{#if d.evidence.length}
					<FormRow label={m.create.evidence}>
						<ul class="flex flex-wrap gap-1.5">
							{#each d.evidence as item (item)}<li><code class="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">{item}</code></li>{/each}
						</ul>
					</FormRow>
				{/if}
				{#if d.apps?.length}
					<FormRow label={m.create.composeApps}>
						<p class="text-sm text-muted-foreground">{m.create.composeAppsHelp}</p>
						<ul class="flex flex-col gap-1 text-sm">
							{#each d.apps as app (app.service)}<li><span class="font-medium">{app.service}</span>: <code class="font-mono text-xs">{m.create.composeApp(app.folder, app.phpVersion, app.documentRoot)}</code></li>{/each}
						</ul>
					</FormRow>
				{/if}
				{#if d.warnings.length}
					<FormRow label={m.create.warnings}>
						<ul class="flex flex-col gap-1 text-sm text-amber-700 dark:text-amber-400">
							{#each d.warnings as item (item)}<li>{item}</li>{/each}
						</ul>
					</FormRow>
				{/if}
				<FormRow label={m.create.name} for="name">
					<Input id="name" class="h-8 max-w-xs" bind:value={flow.name} />
				</FormRow>
				<FormRow label={m.create.preset}>
					<SelectField bind:value={flow.presetId} options={presetOptions} />
				</FormRow>
				<FormRow label={m.create.php}>
					<PhpVersionSelect bind:value={flow.phpVersion} allowDefault />
				</FormRow>
				<FormRow label={m.create.addressing}>
					<SelectField value={flow.addressing} options={addressingOptions} class="w-full max-w-md" onchange={(v) => (flow.addressing = v as Addressing)} />
				</FormRow>
			</Card.Content>
			<Card.Footer class="justify-end">
				<Button type="submit" size="sm" disabled={!flow.canCreate}>{flow.creating ? m.create.creating : m.create.confirm}</Button>
			</Card.Footer>
		</Card.Root>
	{/if}
</form>
