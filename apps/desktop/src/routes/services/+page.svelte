<script lang="ts">
	import Play from '@lucide/svelte/icons/play';
	import RotateCw from '@lucide/svelte/icons/rotate-cw';
	import ShieldQuestion from '@lucide/svelte/icons/shield-question';
	import Square from '@lucide/svelte/icons/square';
	import { onMount } from 'svelte';

	import AsyncBoundary from '#lib/components/common/AsyncBoundary.svelte';
	import PageHeader from '#lib/components/common/PageHeader.svelte';
	import ServiceRow from '#lib/components/services/ServiceRow.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { diagnostics, services } from '#lib/stores/index.js';

	const t = m.services;
	const list = $derived(services.list.data ?? []);
	const conflicts = $derived(diagnostics.doctor.data?.portConflicts ?? []);
	const anyBusy = $derived(Object.keys(services.busy).length > 0);

	onMount(() => {
		void services.load();
		void diagnostics.ensureDoctor();
	});
</script>

<PageHeader title={t.title} subtitle={t.subtitle}>
	{#snippet actions()}
		<Button variant="outline" size="sm" disabled={anyBusy} onclick={() => services.act('start')}><Play />{t.startAll}</Button>
		<Button variant="outline" size="sm" disabled={anyBusy || !services.anyRunning} onclick={() => services.act('restart')}><RotateCw />{t.restartAll}</Button>
		<Button variant="outline" size="sm" disabled={anyBusy || !services.anyRunning} onclick={() => services.act('stop')}><Square />{t.stopAll}</Button>
	{/snippet}
</PageHeader>

<AsyncBoundary status={services.list.status} error={services.list.error} onretry={services.load} empty={list.length === 0} emptyMessage={t.empty} rows={4}>
	<ul class="flex flex-col gap-1.5">
		{#each list as service (service.service)}
			<ServiceRow {service} />
		{/each}
	</ul>
</AsyncBoundary>

<section class="mt-6" aria-labelledby="conflicts-title">
	<div class="mb-2 flex items-center justify-between">
		<h2 id="conflicts-title" class="text-sm font-medium">{t.conflicts}</h2>
		<Button variant="ghost" size="xs" disabled={diagnostics.doctor.refreshing || diagnostics.doctor.status === 'loading'} onclick={diagnostics.runDoctor}><ShieldQuestion />{t.checkConflicts}</Button>
	</div>
	{#if diagnostics.doctor.status === 'error'}
		<p class="text-sm text-muted-foreground">{m.common.loadFailed}</p>
	{:else if conflicts.length === 0}
		<p class="text-sm text-muted-foreground">{diagnostics.doctor.data ? m.settings.doctor.ok : m.app.loading}</p>
	{:else}
		<ul class="flex flex-col gap-1.5">
			{#each conflicts as c (c.port)}
				<li class="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm">{t.conflictBody(c.port, c.wantedBy, c.owner)}</li>
			{/each}
		</ul>
	{/if}
</section>
