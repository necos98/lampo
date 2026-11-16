<script lang="ts">
	import './layout.css';

	import { ModeWatcher } from 'mode-watcher';
	import { onMount } from 'svelte';

	import { goto } from '$app/navigation';
	import { page } from '$app/state';

	import favicon from '#lib/assets/favicon.svg';
	import ElevationBanner from '#lib/components/layout/ElevationBanner.svelte';
	import Sidebar from '#lib/components/layout/Sidebar.svelte';
	import UnsavedChangesGuard from '#lib/components/layout/UnsavedChangesGuard.svelte';
	import { Toaster } from '#lib/components/ui/sonner/index.js';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import { installShortcuts } from '#lib/services/navigation.js';
	import { firstRun, startStores, unsavedChanges, updater } from '#lib/stores/index.js';

	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	/** Icons show their tooltip after a short pause, not on every pass of the pointer. */
	const TOOLTIP_DELAY_MS = 400;

	const inWizard = $derived(page.url.pathname.startsWith('/first-run'));

	onMount(() => {
		let disposeStores: (() => void) | undefined;
		let disposed = false;
		void startStores().then((dispose) => {
			// Unmounted while subscribing: do not leave listeners behind.
			if (disposed) dispose();
			else disposeStores = dispose;
		});
		const disposeShortcuts = installShortcuts({ navigate: (href) => void goto(href), save: () => void unsavedChanges.saveAll() });
		// The tray ("Controlla aggiornamenti") asks a live window to open a route.
		const onNavigate = (e: Event) => {
			const href = (e as CustomEvent<unknown>).detail;
			if (typeof href === 'string' && href.startsWith('/')) void goto(href);
		};
		window.addEventListener('lampo:navigate', onNavigate);
		// Silent: lights the dot on "Impostazioni" when a newer release exists.
		void updater.checkSilently();
		return () => {
			disposed = true;
			disposeStores?.();
			disposeShortcuts();
			window.removeEventListener('lampo:navigate', onNavigate);
		};
	});

	// The wizard gates everything until the first run completed (re-derived from the backend).
	$effect(() => {
		if (firstRun.needsSetup && !inWizard) void goto('/first-run', { replace: true });
	});
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
</svelte:head>

<ModeWatcher />
<Toaster position="bottom-right" />
<UnsavedChangesGuard />

<!-- One provider for every tooltip: once one is open, moving to the next icon shows its tooltip at once. -->
<Tooltip.Provider delayDuration={TOOLTIP_DELAY_MS}>
	{#if inWizard}
		<div class="h-screen overflow-y-auto">{@render children()}</div>
	{:else}
		<div class="flex h-screen overflow-hidden">
			<Sidebar />
			<div class="flex min-w-0 flex-1 flex-col">
				<ElevationBanner />
				<main class="min-h-0 flex-1 overflow-y-auto px-6 py-5">
					{@render children()}
				</main>
			</div>
		</div>
	{/if}
</Tooltip.Provider>
