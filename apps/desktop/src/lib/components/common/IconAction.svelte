<script lang="ts">
	import { mergeProps } from 'bits-ui';
	import type { Snippet } from 'svelte';

	import { Button } from '#lib/components/ui/button/index.js';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';

	interface Props {
		/** Accessible name of the button, also shown as its tooltip. */
		label: string;
		/** Tooltip text when it differs from `label` (e.g. «Copiato» right after a copy). */
		tooltip?: string;
		/** In-app route: renders a link instead of a button. */
		href?: string;
		disabled?: boolean;
		/** Keep the tooltip open after a click, for feedback shown in it. */
		keepTooltipOnClick?: boolean;
		onclick?: () => void;
		/** The icon. */
		children: Snippet;
	}

	let { label, tooltip = label, href, disabled = false, keepTooltipOnClick = false, onclick, children }: Props = $props();
</script>

<!-- Icon-only button: the tooltip (hover or keyboard focus) says what it does. -->
<Tooltip.Root disableCloseOnTriggerClick={keepTooltipOnClick}>
	<Tooltip.Trigger {disabled}>
		{#snippet child({ props })}
			<Button {...mergeProps(props, { onclick })} variant="ghost" size="icon-xs" {href} {disabled} aria-label={label}>
				{@render children()}
			</Button>
		{/snippet}
	</Tooltip.Trigger>
	<Tooltip.Content>{tooltip}</Tooltip.Content>
</Tooltip.Root>
