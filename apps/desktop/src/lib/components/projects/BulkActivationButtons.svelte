<script lang="ts">
	import Play from '@lucide/svelte/icons/play';
	import Square from '@lucide/svelte/icons/square';

	import ConfirmDialog from '#lib/components/common/ConfirmDialog.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { projects } from '#lib/stores/index.js';

	const t = m.projects;

	let confirmOpen = $state(false);
	/** What the open confirmation applies: serve every project (`true`) or stop every one. */
	let target = $state(true);

	const toActivate = $derived(projects.bulkTargetCount(true));
	const toDeactivate = $derived(projects.bulkTargetCount(false));
	const count = $derived(target ? toActivate : toDeactivate);

	function ask(active: boolean) {
		target = active;
		confirmOpen = true;
	}
</script>

<Button variant="outline" size="sm" disabled={projects.bulkToggling || toActivate === 0} onclick={() => ask(true)}><Play />{t.activateAll}</Button>
<Button variant="outline" size="sm" disabled={projects.bulkToggling || toDeactivate === 0} onclick={() => ask(false)}><Square />{t.deactivateAll}</Button>

<ConfirmDialog
	bind:open={confirmOpen}
	title={target ? t.activateAllTitle : t.deactivateAllTitle}
	body={target ? t.activateAllBody(count) : t.deactivateAllBody(count)}
	confirmLabel={target ? t.activateAll : t.deactivateAll}
	onconfirm={() => projects.setAllActive(target)}
/>
