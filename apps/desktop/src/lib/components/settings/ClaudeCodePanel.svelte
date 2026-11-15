<script lang="ts">
	import { onMount } from 'svelte';

	import type { ClaudeCodeStatus, StepOutcome } from '#lib/api/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { integrations } from '#lib/stores/index.js';

	import AsyncBoundary from '../common/AsyncBoundary.svelte';
	import ConfirmDialog from '../common/ConfirmDialog.svelte';
	import CopyField from '../common/CopyField.svelte';

	const t = m.settings.integrations.claude;
	const status = $derived(integrations.claudeCode.data);
	const busy = $derived(integrations.busy !== null);
	let confirmRemove = $state(false);

	const label = (s: ClaudeCodeStatus) => (s.state === 'installed' ? t.state.installed(s.installedVersion ?? s.expectedVersion) : t.state[s.state]);
	const variant = (s: ClaudeCodeStatus) => (s.state === 'installed' ? 'secondary' : s.state === 'outdated' ? 'destructive' : 'outline');
	const outcomeClass: Record<StepOutcome, string> = { ok: 'text-emerald-600 dark:text-emerald-400', failed: 'text-destructive', ignored: 'text-muted-foreground' };

	onMount(() => void integrations.loadClaudeCode());
</script>

<h2 class="text-sm font-medium">{m.settings.integrations.title}</h2>
<div class="mt-2 mb-3">
	<h3 class="text-sm">{t.title}</h3>
	<p class="text-xs text-muted-foreground">{t.help}</p>
</div>
<AsyncBoundary status={integrations.claudeCode.status} error={integrations.claudeCode.error} onretry={integrations.loadClaudeCode} rows={2}>
	{#if status}
		<div class="flex flex-wrap items-center gap-2">
			<Badge variant={variant(status)}>{label(status)}</Badge>
			{#if status.claudeVersion}<span class="text-xs text-muted-foreground">{t.claudeVersion} {status.claudeVersion}</span>{/if}
			<div class="ml-auto flex gap-2">
				<Button size="sm" variant="ghost" disabled={busy || integrations.claudeCode.refreshing} onclick={integrations.loadClaudeCode}>{t.reload}</Button>
				{#if status.state === 'not-installed'}
					<Button size="sm" variant="outline" disabled={busy || !status.cliInstalled} onclick={() => integrations.installClaudeCode()}>
						{integrations.busy === 'install' ? t.installing : t.install}
					</Button>
				{:else if status.state === 'installed' || status.state === 'outdated'}
					{#if status.state === 'outdated'}
						<Button size="sm" variant="outline" disabled={busy || !status.cliInstalled} onclick={() => integrations.installClaudeCode()}>
							{integrations.busy === 'install' ? t.installing : t.update}
						</Button>
					{/if}
					<Button size="sm" variant="outline" disabled={busy} onclick={() => (confirmRemove = true)}>
						{integrations.busy === 'uninstall' ? t.removing : t.uninstall}
					</Button>
				{/if}
			</div>
		</div>
		{#if status.state === 'isolated'}
			<p class="mt-2 text-xs text-muted-foreground">{t.isolatedHelp}</p>
		{:else if status.state === 'claude-missing'}
			<p class="mt-2 text-xs text-muted-foreground">{t.missingHelp}</p>
		{:else if !status.cliInstalled && status.state !== 'installed'}
			<p class="mt-2 rounded border border-amber-500/40 bg-amber-500/10 px-2.5 py-1.5 text-xs">{t.cliMissing}</p>
		{/if}
		{#if status.updateError}
			<p class="mt-2 rounded border border-amber-500/40 bg-amber-500/10 px-2.5 py-1.5 text-xs">{t.updateError(status.updateError)}</p>
		{/if}
		{#if status.state !== 'isolated' && status.state !== 'claude-missing'}
			<div class="mt-2"><CopyField label={t.pluginDir} value={status.pluginDir} /></div>
		{/if}
		{#if integrations.steps.length}
			<details class="mt-3 text-xs" open={integrations.lastOk === false}>
				<summary class="cursor-pointer text-muted-foreground">{t.log}</summary>
				<ol class="mt-1 flex flex-col gap-1">
					{#each integrations.steps as s, i (i)}
						<li>
							<span class={outcomeClass[s.outcome]}>{t.outcome[s.outcome]}</span>
							<span class="ml-1">{t.steps[s.step] ?? s.step}</span>
							{#if s.detail}<pre class="mt-0.5 overflow-x-auto font-mono whitespace-pre-wrap text-muted-foreground">{s.detail}</pre>{/if}
						</li>
					{/each}
				</ol>
			</details>
		{/if}
	{/if}
</AsyncBoundary>

<ConfirmDialog
	bind:open={confirmRemove}
	title={t.confirmRemove.title}
	body={t.confirmRemove.body}
	confirmLabel={t.confirmRemove.confirm}
	destructive
	onconfirm={() => {
		confirmRemove = false;
		void integrations.uninstallClaudeCode();
	}}
/>
