<script lang="ts">
	import ScrollText from '@lucide/svelte/icons/scroll-text';

	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { projectLogsHref, SITE_LOG_KINDS } from '#lib/projects/links.js';

	let { site }: { site: string } = $props();

	const t = m.projects;
</script>

<div class="flex items-center gap-0.5" role="group" aria-label={t.logs}>
	<ScrollText class="mr-1 size-4 text-muted-foreground" aria-hidden="true" />
	<span class="mr-1 text-sm">{t.logs}</span>
	{#each SITE_LOG_KINDS as kind (kind)}
		<Button variant="ghost" size="sm" href={projectLogsHref(site, kind)} title={t.logLinkHelp[kind]}>{t.logLinks[kind]}</Button>
	{/each}
</div>
