<script lang="ts" module>
	export interface Option {
		value: string;
		label: string;
		disabled?: boolean;
	}
</script>

<script lang="ts">
	import * as Select from '#lib/components/ui/select/index.js';

	interface Props {
		value: string;
		options: Option[];
		placeholder?: string;
		id?: string;
		disabled?: boolean;
		class?: string;
		/** Accessible name when no visible <label for> points at the trigger. */
		'aria-label'?: string;
		onchange?: (value: string) => void;
	}

	let { value = $bindable(), options, placeholder = '', id, disabled = false, class: className = 'w-56', 'aria-label': ariaLabel, onchange }: Props = $props();
	const current = $derived(options.find((o) => o.value === value)?.label ?? placeholder);
</script>

<Select.Root type="single" bind:value {disabled} onValueChange={(v) => onchange?.(v)}>
	<Select.Trigger {id} size="sm" class={className} aria-label={ariaLabel}>{current}</Select.Trigger>
	<Select.Content>
		{#each options as option (option.value)}
			<Select.Item value={option.value} label={option.label} disabled={option.disabled}>{option.label}</Select.Item>
		{/each}
	</Select.Content>
</Select.Root>
