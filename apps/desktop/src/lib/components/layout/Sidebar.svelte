<script lang="ts">
	import Boxes from '@lucide/svelte/icons/boxes';
	import FolderGit2 from '@lucide/svelte/icons/folder-git-2';
	import Package from '@lucide/svelte/icons/package';
	import ScrollText from '@lucide/svelte/icons/scroll-text';
	import Settings from '@lucide/svelte/icons/settings';

	import { page } from '$app/state';

	import logo from '#lib/assets/favicon.svg';
	import { messages as m } from '#lib/messages/index.js';
	import { NAV_ITEMS } from '#lib/services/navigation.js';
	import { services, simulated, updater } from '#lib/stores/index.js';

	import StatusDot from '../common/StatusDot.svelte';

	const icons = { projects: FolderGit2, services: Boxes, packages: Package, logs: ScrollText, settings: Settings };

	const running = $derived((services.list.data ?? []).filter((s) => s.state === 'running').length);
	const total = $derived(services.list.data?.length ?? 0);
	const tone = $derived((services.list.data ?? []).some((s) => s.state === 'failed') ? 'error' : running > 0 ? 'ok' : 'idle');
</script>

<nav class="flex w-48 shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground" aria-label={m.nav.label}>
	<div class="flex items-center gap-2 px-4 py-3">
		<img src={logo} alt="" class="size-5" />
		<span class="font-semibold tracking-tight">{m.app.name}</span>
		{#if simulated}
			<span class="ml-auto rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400" title={m.app.simulated}>{m.app.mockBadge}</span>
		{/if}
	</div>
	<ul class="flex flex-1 flex-col gap-0.5 px-2">
		{#each NAV_ITEMS as item, i (item.id)}
			{@const Icon = icons[item.id]}
			{@const active = page.url.pathname === item.href || page.url.pathname.startsWith(`${item.href}/`)}
			<li>
				<a
					href={item.href}
					aria-current={active ? 'page' : undefined}
					title="Alt+{i + 1}"
					class="flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm outline-none transition-colors hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-sidebar-ring {active ? 'bg-sidebar-accent font-medium text-sidebar-accent-foreground' : 'text-muted-foreground'}"
				>
					<Icon class="size-4" />
					{m.nav[item.id]}
					{#if item.id === 'settings' && updater.available}
						<span class="ml-auto size-2 rounded-full bg-primary" role="status" aria-label={m.settings.updates.dot} title={m.settings.updates.dot}></span>
					{/if}
				</a>
			</li>
		{/each}
	</ul>
	<a href="/services" class="flex items-center gap-2 border-t px-4 py-2.5 text-xs text-muted-foreground hover:text-foreground">
		<StatusDot {tone} />
		{#if total > 0}{m.services.states.running}: {running}/{total}{:else}{m.nav.services}{/if}
	</a>
</nav>
