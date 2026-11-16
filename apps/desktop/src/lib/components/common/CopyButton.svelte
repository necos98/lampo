<script lang="ts">
	import Check from '@lucide/svelte/icons/check';
	import Copy from '@lucide/svelte/icons/copy';

	import { messages as m } from '#lib/messages/index.js';
	import { notifier } from '#lib/services/notifier.js';
	import { copyText } from '#lib/services/platform.js';

	import IconAction from './IconAction.svelte';

	/** How long the check mark and «Copiato» stay after a copy. */
	const COPIED_FEEDBACK_MS = 1200;

	interface Props {
		value: string;
		/** Accessible name and tooltip (`Copia URL`). */
		label: string;
	}

	let { value, label }: Props = $props();
	let copied = $state(false);
	let timer: ReturnType<typeof setTimeout> | undefined;

	$effect(() => () => clearTimeout(timer));

	async function copy() {
		if (await copyText(value)) {
			copied = true;
			clearTimeout(timer);
			timer = setTimeout(() => (copied = false), COPIED_FEEDBACK_MS);
		} else notifier.error(m.common.copyFailed);
	}
</script>

<IconAction {label} tooltip={copied ? m.common.copied : label} keepTooltipOnClick onclick={copy}>
	{#if copied}<Check />{:else}<Copy />{/if}
</IconAction>
<span class="sr-only" role="status">{copied ? m.common.copied : ''}</span>
