<script lang="ts">
	import ExternalLink from '@lucide/svelte/icons/external-link';
	import Play from '@lucide/svelte/icons/play';
	import RotateCw from '@lucide/svelte/icons/rotate-cw';
	import Square from '@lucide/svelte/icons/square';

	import type { ServiceState, ServiceStatus } from '#lib/api/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { describeError, errorOutput } from '#lib/services/errors.js';
	import { openExternal } from '#lib/services/platform.js';
	import { services } from '#lib/stores/index.js';

	import StatusDot, { type Tone } from '../common/StatusDot.svelte';

	let { service }: { service: ServiceStatus } = $props();
	const t = m.services;

	const tones: Record<ServiceState, Tone> = { running: 'ok', starting: 'busy', stopping: 'busy', stopped: 'idle', failed: 'error', unknown: 'idle' };
	const label = $derived(service.label || service.service);
	const busy = $derived(services.busy[service.service] === true || service.state === 'starting' || service.state === 'stopping');
	const running = $derived(service.state === 'running');
	const stateLabel = $derived(service.state === 'starting' && service.message ? t.startingWith(service.message) : t.states[service.state]);
	const failure = $derived(services.failures[service.service]);
	const reason = $derived(service.required && service.requiredBy.length === 0 ? t.requiredBySystem : service.requiredBy.length ? t.requiredBy(service.requiredBy) : t.unused);
</script>

<li class="rounded-md border bg-card px-3 py-2.5">
	<div class="grid grid-cols-[auto_minmax(0,1fr)_6rem_minmax(5rem,auto)_auto] items-center gap-x-4">
		<StatusDot tone={tones[service.state]} label={stateLabel} />
		<div class="min-w-0">
			<p class="font-medium">{label}</p>
			<p class="truncate text-xs text-muted-foreground">{reason}</p>
		</div>
		<span class="text-sm {service.state === 'failed' ? 'text-destructive' : 'text-muted-foreground'}">{stateLabel}</span>
		<span class="font-mono text-xs text-muted-foreground">{service.port ? `:${service.port}` : ''}{service.uiPort ? ` · ui :${service.uiPort}` : ''}{service.pid ? ` · ${service.pid}` : ''}</span>
		<div class="flex gap-1">
			{#if service.uiUrl}
				<!-- The web UI (Mailpit's caught mail) opens in the default browser. -->
				<Button variant="ghost" size="icon-xs" disabled={!running} title={t.openUi(label)} onclick={() => openExternal(service.uiUrl ?? '')} aria-label={t.openUi(label)}><ExternalLink /></Button>
			{/if}
			{#if running}
				<Button variant="ghost" size="icon-xs" disabled={busy} onclick={() => services.act('restart', [service.service])} aria-label="{t.restart} {label}"><RotateCw /></Button>
				<Button variant="ghost" size="icon-xs" disabled={busy} onclick={() => services.act('stop', [service.service])} aria-label="{t.stop} {label}"><Square /></Button>
			{:else}
				<Button variant="ghost" size="icon-xs" disabled={busy} onclick={() => services.act('start', [service.service])} aria-label="{t.start} {label}"><Play /></Button>
			{/if}
		</div>
	</div>
	{#if service.state === 'failed' || failure}
		<div class="mt-2 rounded border border-destructive/40 bg-destructive/10 px-2.5 py-1.5 text-xs" role="alert">
			{#if failure}<p>{describeError(failure)}</p>{:else if service.message}<p>{service.message}</p>{/if}
			{#if failure && errorOutput(failure)}<pre class="mt-1 whitespace-pre-wrap font-mono">{errorOutput(failure)}</pre>{/if}
		</div>
	{/if}
</li>
