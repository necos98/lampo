<script lang="ts">
	import Check from '@lucide/svelte/icons/check';
	import Copy from '@lucide/svelte/icons/copy';
	import Eye from '@lucide/svelte/icons/eye';
	import EyeOff from '@lucide/svelte/icons/eye-off';

	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { notifier } from '#lib/services/notifier.js';
	import { copyText } from '#lib/services/platform.js';

	interface Props {
		label: string;
		value: string;
		/** Mask the value until revealed (passwords). */
		secret?: boolean;
	}

	let { label, value, secret = false }: Props = $props();
	let revealed = $state(false);
	let copied = $state(false);

	async function copy() {
		if (await copyText(value)) {
			copied = true;
			setTimeout(() => (copied = false), 1200);
		} else notifier.error(m.common.copyFailed);
	}

	const shown = $derived(value === '' ? m.common.emptyPassword : secret && !revealed ? '••••••••' : value);
</script>

<div class="flex items-center gap-2 text-sm">
	<span class="w-24 shrink-0 text-muted-foreground">{label}</span>
	<code class="min-w-0 flex-1 truncate rounded bg-muted px-2 py-1 font-mono text-xs">{shown}</code>
	{#if secret && value !== ''}
		<Button variant="ghost" size="icon-xs" onclick={() => (revealed = !revealed)} aria-label={revealed ? m.common.hide : m.common.show}>
			{#if revealed}<EyeOff />{:else}<Eye />{/if}
		</Button>
	{/if}
	<Button variant="ghost" size="icon-xs" onclick={copy} aria-label="{m.common.copy} {label}">
		{#if copied}<Check />{:else}<Copy />{/if}
	</Button>
</div>
