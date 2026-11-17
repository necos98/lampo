<script lang="ts">
	import Play from '@lucide/svelte/icons/play';
	import Square from '@lucide/svelte/icons/square';

	import ConfirmDialog from '#lib/components/common/ConfirmDialog.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { BulkConfirmation } from '#lib/projects/bulkConfirmation.svelte.js';
	import { projects } from '#lib/stores/index.js';

	const t = m.projects;

	const confirmation = new BulkConfirmation((active) => projects.bulkTargetCount(active));

	const toActivate = $derived(projects.bulkTargetCount(true));
	const toDeactivate = $derived(projects.bulkTargetCount(false));
</script>

<Button variant="outline" size="sm" disabled={projects.bulkToggling || toActivate === 0} onclick={() => confirmation.ask(true)}><Play />{t.activateAll}</Button>
<Button variant="outline" size="sm" disabled={projects.bulkToggling || toDeactivate === 0} onclick={() => confirmation.ask(false)}><Square />{t.deactivateAll}</Button>

<ConfirmDialog
	bind:open={confirmation.open}
	title={confirmation.target ? t.activateAllTitle : t.deactivateAllTitle}
	body={confirmation.target ? t.activateAllBody(confirmation.count) : t.deactivateAllBody(confirmation.count)}
	confirmLabel={confirmation.target ? t.activateAll : t.deactivateAll}
	onconfirm={() => projects.setAllActive(confirmation.target)}
/>
