<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { scanExclusion } from '#lib/stores/index.js';

	interface Props {
		/** The executables Lampo excluded that are no longer installed (never empty: the panel hides this otherwise). */
		paths: readonly string[];
	}

	let { paths }: Props = $props();

	const t = m.settings.defender;
	const pending = $derived(scanExclusion.busy);
	const removing = $derived(pending?.target === 'uninstalled');
</script>

<section class="mt-4" aria-labelledby="defender-uninstalled">
	<h3 id="defender-uninstalled" class="text-sm font-medium">{t.uninstalled.title}</h3>
	<p class="text-xs text-muted-foreground">{t.uninstalled.help}</p>
	<p class="mt-2 text-xs text-muted-foreground">{t.uninstalled.targets}</p>
	<ul class="mt-1 space-y-1">
		{#each paths as path (path)}
			<li><code class="block truncate rounded bg-muted px-2 py-1 font-mono text-xs" title={path}>{path}</code></li>
		{/each}
	</ul>
	<div class="mt-2 flex justify-end">
		<Button size="sm" variant="outline" disabled={pending !== null} onclick={() => scanExclusion.removeUninstalled()}>{removing ? t.removing : t.uninstalled.remove}</Button>
	</div>
</section>
