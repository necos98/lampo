<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';

	interface Props {
		dirty: boolean;
		valid?: boolean;
		saving: boolean;
		onsave: () => void;
		onreset: () => void;
	}

	let { dirty, valid = true, saving, onsave, onreset }: Props = $props();
</script>

{#if dirty}
	<div class="sticky bottom-0 -mx-6 mt-4 flex items-center justify-end gap-3 border-t bg-background/95 px-6 py-3 backdrop-blur" role="region" aria-label={m.common.unsaved}>
		<span class="mr-auto text-sm text-muted-foreground">{m.common.unsaved}</span>
		<Button variant="ghost" size="sm" onclick={onreset} disabled={saving}>{m.common.discard}</Button>
		<Button size="sm" onclick={onsave} disabled={saving || !valid}>{saving ? m.common.saving : m.common.save}</Button>
	</div>
{/if}
