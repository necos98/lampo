<script lang="ts">
	import RefreshCw from '@lucide/svelte/icons/refresh-cw';
	import { onMount } from 'svelte';

	import AsyncBoundary from '#lib/components/common/AsyncBoundary.svelte';
	import PageHeader from '#lib/components/common/PageHeader.svelte';
	import PackageRowItem from '#lib/components/packages/PackageRowItem.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Tabs from '#lib/components/ui/tabs/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { groupsOf } from '#lib/packages/grouping.js';
	import { packages } from '#lib/stores/index.js';

	const t = m.packages;
	/** Display order of the groups the GUI has labels for; the backend decides which group a package is in. */
	const PREFERRED = Object.keys(t.tabs);
	/** Tabs = the groups present in the catalog; a group the GUI does not know yet gets its own tab. */
	// While the catalog loads (or fails) one tab carries the loading/error state.
	const tabs = $derived(packages.rows.length ? groupsOf(packages.rows, PREFERRED) : [PREFERRED[0]]);
	const tabLabel = (group: string) => (t.tabs as Record<string, string>)[group] ?? t.kinds[group] ?? group;

	let tab = $state<string>('php');
	const updates = $derived(packages.updates.data ?? []);
	const checking = $derived(packages.updates.refreshing || packages.updates.status === 'loading');

	onMount(() => {
		void packages.loadLocal();
		void packages.checkUpdates();
	});

	const rowsFor = (group: string) => packages.rowsOfGroup(group);
	const status = $derived(packages.catalog.status);
</script>

<PageHeader title={t.title} subtitle={t.subtitle}>
	{#snippet actions()}
		{#if packages.updates.status === 'error'}<span class="text-xs text-destructive">{t.updatesFailed}</span>
		{:else if packages.updates.data}<span class="text-xs text-muted-foreground">{updates.length ? t.updatesAvailable(updates.length) : t.upToDate}</span>{/if}
		<Button variant="outline" size="sm" disabled={checking} onclick={packages.checkUpdates}><RefreshCw class={checking ? 'animate-spin' : ''} />{checking ? t.checkingUpdates : t.checkUpdates}</Button>
	{/snippet}
</PageHeader>

<Tabs.Root bind:value={tab}>
	<Tabs.List>
		{#each tabs as id (id)}
			<Tabs.Trigger value={id}>{tabLabel(id)}</Tabs.Trigger>
		{/each}
	</Tabs.List>
	{#each tabs as id (id)}
		<Tabs.Content value={id} class="pt-3">
			{@const rows = rowsFor(id)}
			<AsyncBoundary {status} error={packages.catalog.error} onretry={packages.loadLocal} empty={rows.length === 0} emptyMessage={t.empty} rows={6}>
				<ul class="flex flex-col gap-1.5">
					{#each rows as row (row.key)}
						<PackageRowItem {row} />
					{/each}
				</ul>
			</AsyncBoundary>
		</Tabs.Content>
	{/each}
</Tabs.Root>
