<script lang="ts">
	import ShieldAlert from '@lucide/svelte/icons/shield-alert';

	import type { Addressing } from '#lib/api/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { ADDRESSING_MODES, needsElevation } from '#lib/projects/addressing.js';

	interface Props {
		value: Addressing;
		/** Visible title of the group (its `<legend>`). */
		legend: string;
		help?: string;
		disabled?: boolean;
		onchange: (value: Addressing) => void;
	}

	let { value, legend, help, disabled = false, onchange }: Props = $props();
	// Radios of one group share a name; two groups on a page must not.
	const name = $props.id();
	const t = m.addressing;
</script>

<fieldset {disabled} aria-describedby={help ? `${name}-help` : undefined} class="flex flex-col gap-2 py-2">
	<legend class="mb-2 text-sm text-muted-foreground">{legend}</legend>
	{#each ADDRESSING_MODES as mode (mode)}
		<label
			class="flex cursor-pointer gap-3 rounded-lg border p-3 transition-colors hover:bg-muted/50 has-checked:border-primary has-checked:bg-primary/5 has-checked:ring-1 has-checked:ring-primary has-focus-visible:outline-2 has-focus-visible:outline-ring has-disabled:cursor-default has-disabled:opacity-50"
		>
			<input type="radio" {name} value={mode} checked={value === mode} onchange={() => onchange(mode)} class="mt-0.5 accent-primary" />
			<span class="flex flex-col gap-0.5">
				<span class="text-sm font-medium">{t.modes[mode].title}</span>
				<span class="text-xs text-muted-foreground">{t.modes[mode].description}</span>
				{#if needsElevation(mode)}
					<span class="mt-0.5 flex items-center gap-1 text-xs text-amber-700 dark:text-amber-400"><ShieldAlert class="size-3.5" />{t.needsAdmin}</span>
				{/if}
			</span>
		</label>
	{/each}
	{#if help}<p id="{name}-help" class="text-xs text-muted-foreground">{help}</p>{/if}
</fieldset>
