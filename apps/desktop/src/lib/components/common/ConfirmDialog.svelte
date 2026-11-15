<script lang="ts">
	import type { Snippet } from 'svelte';

	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';

	interface Props {
		open: boolean;
		title: string;
		body: string;
		confirmLabel: string;
		destructive?: boolean;
		onconfirm: () => void | Promise<void>;
		/** Extra controls between the text and the buttons (e.g. a checkbox). */
		children?: Snippet;
	}

	let { open = $bindable(), title, body, confirmLabel, destructive = false, onconfirm, children }: Props = $props();
</script>

<AlertDialog.Root bind:open>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{title}</AlertDialog.Title>
			<AlertDialog.Description>{body}</AlertDialog.Description>
		</AlertDialog.Header>
		{@render children?.()}
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{m.common.cancel}</AlertDialog.Cancel>
			<AlertDialog.Action class={destructive ? buttonVariants({ variant: 'destructive' }) : undefined} onclick={() => onconfirm()}>{confirmLabel}</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
