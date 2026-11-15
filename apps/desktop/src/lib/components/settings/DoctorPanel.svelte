<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Stethoscope from '@lucide/svelte/icons/stethoscope';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';

	import type { CheckStatus } from '#lib/api/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { diagnostics } from '#lib/stores/index.js';

	import ErrorNotice from '../common/ErrorNotice.svelte';
	import ExternalLink from '../common/ExternalLink.svelte';

	const t = m.settings.doctor;
	const report = $derived(diagnostics.doctor.data);
	const running = $derived(diagnostics.doctor.status === 'loading' || diagnostics.doctor.refreshing);

	const icons: Record<CheckStatus, typeof CircleCheck> = { ok: CircleCheck, warning: TriangleAlert, error: CircleAlert };
	const colours: Record<CheckStatus, string> = { ok: 'text-emerald-500', warning: 'text-amber-500', error: 'text-destructive' };
</script>

<div class="flex items-center justify-between gap-3">
	<div>
		<h2 class="text-sm font-medium">{t.title}</h2>
		<p class="text-xs text-muted-foreground">{t.help}</p>
	</div>
	<Button size="sm" variant="outline" disabled={running} onclick={diagnostics.runDoctor}><Stethoscope />{running ? t.running : t.run}</Button>
</div>

<div class="mt-3">
	{#if diagnostics.doctor.status === 'error'}
		<ErrorNotice error={diagnostics.doctor.error} onretry={diagnostics.runDoctor} />
	{:else if !report}
		<p class="text-sm text-muted-foreground">{running ? t.running : t.notRun}</p>
	{:else}
		<p class="mb-2 text-sm {report.ok ? 'text-emerald-600 dark:text-emerald-400' : 'text-destructive'}">{report.ok ? t.ok : t.problems}</p>
		<ul class="flex flex-col gap-1">
			{#each report.checks as check (check.id)}
				{@const Icon = icons[check.status]}
				{@const summary = t.outcomes[check.id]?.[check.status]}
				<li class="flex items-start gap-2 rounded-md border bg-card px-3 py-2 text-sm">
					<Icon class="mt-0.5 size-4 shrink-0 {colours[check.status]}" aria-label={t.status[check.status]} />
					<div class="min-w-0">
						<p>{t.checks[check.id] ?? check.id}{#if summary}<span class="text-muted-foreground">&nbsp;— {summary}</span>{/if}</p>
						{#if !summary}
							<p class="text-xs text-muted-foreground">{check.message}</p>
						{:else if check.status !== 'ok'}
							<p class="text-xs text-muted-foreground" title={t.technical}>{check.message}</p>
						{/if}
						{#if check.fix && check.status !== 'ok'}<p class="font-mono text-xs text-muted-foreground">{t.fix}: {check.fix}</p>{/if}
					</div>
				</li>
			{/each}
		</ul>
		{#if report.missingVcRuntimes.length}
			<h3 class="mt-4 mb-1 text-sm font-medium">{t.vcRuntime}</h3>
			<ul class="flex flex-col gap-1">
				{#each report.missingVcRuntimes as vc (vc.redistributable)}
					<li class="flex items-center gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm">
						<span class="flex-1">{vc.redistributable} <span class="text-xs text-muted-foreground">({vc.requiredBy.join(', ')})</span></span>
						<ExternalLink href={vc.downloadUrl}>{t.download}</ExternalLink>
					</li>
				{/each}
			</ul>
		{/if}
		{#if report.staleWorktrees.length}
			<h3 class="mt-4 mb-1 text-sm font-medium">{t.staleWorktrees}</h3>
			<p class="font-mono text-xs text-muted-foreground">{report.staleWorktrees.join(', ')}</p>
		{/if}
		{#if report.missingPackages.length}
			<h3 class="mt-4 mb-1 text-sm font-medium">{t.missingPackages}</h3>
			<p class="font-mono text-xs text-muted-foreground">{report.missingPackages.join(', ')}</p>
		{/if}
	{/if}
</div>
