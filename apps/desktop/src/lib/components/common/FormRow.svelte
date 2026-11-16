<script lang="ts">
	import type { Snippet } from 'svelte';

	import { Label } from '#lib/components/ui/label/index.js';

	import { type FormFieldAttrs, formFieldIds } from './formField.js';

	interface Props {
		label: string;
		/** Id of the control; without it the row generates one, handed to `children`. */
		for?: string;
		help?: string;
		/** Receives the label and help wiring, to spread on the control: `<Input {...field} />`. */
		children: Snippet<[FormFieldAttrs]>;
	}

	let { label, for: forId, help, children }: Props = $props();
	const uid = $props.id();
	const ids = $derived(formFieldIds(uid, forId, !!help));
</script>

<div class="grid grid-cols-[11rem_1fr] items-start gap-x-4 gap-y-1 py-2">
	<Label for={ids.field.id} class="pt-1.5 text-sm font-normal text-muted-foreground">{label}</Label>
	<div class="min-w-0">
		{@render children(ids.field)}
		{#if help}<p id={ids.helpId} class="mt-1 text-xs text-muted-foreground">{help}</p>{/if}
	</div>
</div>
