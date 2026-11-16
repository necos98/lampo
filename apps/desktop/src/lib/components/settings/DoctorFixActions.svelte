<script lang="ts">
	import type { DoctorCheck, DoctorReport } from '#lib/api/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { fixActions, type FixAction, type FixCommand } from '#lib/diagnostics/fixes.js';
	import { messages as m } from '#lib/messages/index.js';
	import { diagnostics, elevation, services, worktrees } from '#lib/stores/index.js';

	import ConfirmDialog from '../common/ConfirmDialog.svelte';
	import ExternalLink from '../common/ExternalLink.svelte';

	interface Props {
		check: DoctorCheck;
		report: DoctorReport;
	}

	let { check, report }: Props = $props();

	const t = m.settings.doctor;
	const actions = $derived(fixActions(check, report));
	const offersPrune = $derived(actions.some((a) => a.kind === 'command' && a.command === 'prune-worktrees'));
	let pruneOpen = $state(false);

	const busy: Record<FixCommand, boolean> = $derived({
		'install-shims': diagnostics.installingShims,
		'apply-hosts': elevation.applying,
		'start-services': report.servicesDown.some((s) => services.busy[s]),
		// The confirmation dialog shows its own progress.
		'prune-worktrees': false
	});

	const key = (action: FixAction) => (action.kind === 'command' ? action.command : action.href);

	// Installing the shims reloads the report by itself; the other fixes refresh it here.
	const run: Record<FixCommand, () => void> = {
		'install-shims': () => void diagnostics.installShims(),
		'apply-hosts': () => void elevation.apply().then(diagnostics.runDoctor),
		// A failed service is started again; none listed (states unreadable) starts the whole plan.
		'start-services': () => void services.act('start', report.servicesDown).then(diagnostics.runDoctor),
		'prune-worktrees': () => (pruneOpen = true)
	};

	async function prune() {
		const result = await worktrees.prune(null, false);
		if (result.ok) void diagnostics.runDoctor();
		return result;
	}
</script>

{#if actions.length}
	<div class="mt-1.5 flex flex-wrap items-center gap-2">
		{#each actions as action (key(action))}
			{#if action.kind === 'command'}
				<Button size="xs" variant="outline" disabled={busy[action.command]} onclick={run[action.command]}>{t.actions[action.command]}</Button>
			{:else if action.kind === 'navigate'}
				<Button size="xs" variant="outline" href={action.href}>{t.actions.open(m.nav[action.page])}</Button>
			{:else}
				<ExternalLink href={action.href}>{t.actions.download(action.label)}</ExternalLink>
			{/if}
		{/each}
	</div>
{/if}

{#if offersPrune}
	<ConfirmDialog
		bind:open={pruneOpen}
		title={t.pruneTitle}
		body="{t.pruneBody} {report.staleWorktrees.join(', ')}"
		confirmLabel={t.actions['prune-worktrees']}
		destructive
		onconfirm={prune}
	/>
{/if}
