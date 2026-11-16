<script lang="ts">
	import { beforeNavigate, goto } from '$app/navigation';

	import ConfirmDialog from '#lib/components/common/ConfirmDialog.svelte';
	import { messages as m } from '#lib/messages/index.js';
	import { unsavedChanges } from '#lib/stores/index.js';

	const t = m.unsaved;

	let open = $state(false);
	/** Where the user was going when the dialog stopped them. */
	let destination: URL | null = null;

	// Every in-app navigation passes here: sidebar links, Alt+1..5, back/forward, the tray.
	beforeNavigate((navigation) => {
		// Closing the window is not an in-app navigation; a URL change on the same page (a query,
		// a hash) keeps the form mounted, so no edit is lost.
		if (navigation.type === 'leave' || !navigation.to || navigation.to.url.pathname === navigation.from?.url.pathname) return;
		if (!unsavedChanges.dirty) return;
		navigation.cancel();
		destination = navigation.to.url;
		open = true;
	});

	function discard() {
		const target = destination;
		destination = null;
		unsavedChanges.discardAll();
		if (target) void goto(target);
	}
</script>

<ConfirmDialog bind:open title={t.leaveTitle} body={t.leaveBody} confirmLabel={t.discard} cancelLabel={t.stay} destructive onconfirm={discard} />
