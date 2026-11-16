<script lang="ts">
	import LoaderCircle from '@lucide/svelte/icons/loader-circle';
	import type { Snippet } from 'svelte';

	import { toAppError, type AppError } from '#lib/api/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import type { Outcome } from '#lib/stores/runner.js';

	import ErrorNotice from './ErrorNotice.svelte';

	/** A failed `Outcome` keeps the dialog open; anything else (a success, nothing to do) closes it. */
	type ConfirmResult = Outcome<unknown> | null | void;

	interface Props {
		open: boolean;
		title: string;
		body: string;
		confirmLabel: string;
		destructive?: boolean;
		onconfirm: () => ConfirmResult | Promise<ConfirmResult>;
		/** Extra controls between the text and the buttons (e.g. a checkbox). */
		children?: Snippet;
	}

	let { open = $bindable(), title, body, confirmLabel, destructive = false, onconfirm, children }: Props = $props();

	let busy = $state(false);
	let error = $state<AppError | null>(null);

	// AlertDialog.Action does not close the dialog by itself: it closes only once the action succeeded.
	async function confirm() {
		if (busy) return;
		busy = true;
		error = null;
		try {
			const result = await onconfirm();
			if (result && !result.ok) error = result.error;
			else open = false;
		} catch (e) {
			error = toAppError(e);
		} finally {
			busy = false;
		}
	}
</script>

<AlertDialog.Root bind:open onOpenChangeComplete={(isOpen) => !isOpen && (error = null)}>
	<AlertDialog.Content onEscapeKeydown={(e) => busy && e.preventDefault()} aria-busy={busy}>
		<AlertDialog.Header>
			<AlertDialog.Title>{title}</AlertDialog.Title>
			<AlertDialog.Description>{body}</AlertDialog.Description>
		</AlertDialog.Header>
		{@render children?.()}
		{#if error}<ErrorNotice {error} title={m.common.actionFailed} />{/if}
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={busy}>{m.common.cancel}</AlertDialog.Cancel>
			<AlertDialog.Action class={destructive ? buttonVariants({ variant: 'destructive' }) : undefined} disabled={busy} onclick={confirm}>
				{#if busy}<LoaderCircle class="animate-spin" />{/if}{confirmLabel}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
