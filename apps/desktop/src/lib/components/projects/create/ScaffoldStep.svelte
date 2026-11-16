<script lang="ts">
	import Play from '@lucide/svelte/icons/play';

	import FormRow from '#lib/components/common/FormRow.svelte';
	import PathInput from '#lib/components/common/PathInput.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { describeError } from '#lib/services/errors.js';
	import type { CreateProjectFlow } from '#lib/stores/createProject.svelte.js';

	import LaravelOptions from './LaravelOptions.svelte';

	/** The form of a new folder (git clone or Laravel): what to set up and where. */
	let { flow }: { flow: CreateProjectFlow } = $props();
	const t = m.create;

	const locked = $derived(flow.phase === 'running' || flow.phase === 'done');
	const folderMessages = { empty: t.folderEmpty, invalid: t.folderInvalid, 'laravel-name': t.laravelNameInvalid } as const;
	// Only once something was typed: an untouched form shows no error.
	const folderMessage = $derived(flow.folderProblem && (flow.folderName.trim() || flow.parentDir.trim()) ? folderMessages[flow.folderProblem] : null);
</script>

<Card.Root size="sm">
	<Card.Header>
		<Card.Title>{t.stepSource}</Card.Title>
		{#if flow.source === 'laravel'}<Card.Description>{t.installerNote}</Card.Description>{/if}
	</Card.Header>
	<Card.Content class="flex flex-col gap-1">
		<fieldset disabled={locked} class="contents">
			{#if flow.source === 'git'}
				<FormRow label={t.repoUrl} help={t.repoUrlHelp}>
					{#snippet children(field)}
						<Input {...field} class="h-8" placeholder={t.repoUrlPlaceholder} bind:value={flow.repoUrl} oninput={() => flow.repoUrlEdited()} autocomplete="off" spellcheck={false} />
					{/snippet}
				</FormRow>
				<FormRow label={t.branch} for="branch">
					<Input id="branch" class="h-8 max-w-xs" placeholder={t.branchPlaceholder} bind:value={flow.branch} autocomplete="off" spellcheck={false} />
				</FormRow>
			{:else}
				<FormRow label={t.laravelName} help={t.laravelNameHelp}>
					{#snippet children(field)}
						<Input {...field} class="h-8 max-w-xs" placeholder="mio-progetto" bind:value={flow.folderName} oninput={() => flow.folderNameEdited()} autocomplete="off" spellcheck={false} />
					{/snippet}
				</FormRow>
			{/if}

			<FormRow label={t.parentDir} help={t.parentDirHelp}>
				{#snippet children(field)}
					<PathInput {...field} mode="folder" title={t.parentDir} placeholder={t.parentDirPlaceholder} bind:value={flow.parentDir} />
				{/snippet}
			</FormRow>

			{#if flow.source === 'git'}
				<FormRow label={t.folderName} help={t.folderNameHelp}>
					{#snippet children(field)}
						<Input {...field} class="h-8 max-w-xs" bind:value={flow.folderName} oninput={() => flow.folderNameEdited()} autocomplete="off" spellcheck={false} />
					{/snippet}
				</FormRow>
			{:else}
				<LaravelOptions {flow} />
			{/if}
		</fieldset>

		{#if folderMessage}
			<p class="text-xs text-destructive" role="alert">{folderMessage}</p>
		{:else if flow.destination}
			<p class="text-xs text-muted-foreground">{t.destination(flow.destination)}</p>
		{/if}
		{#if flow.phase === 'form' && flow.scaffoldError}
			<p class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive" role="alert">
				{describeError(flow.scaffoldError)}<span class="mt-1 block font-mono text-xs opacity-80">{flow.scaffoldError.message}</span>
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
