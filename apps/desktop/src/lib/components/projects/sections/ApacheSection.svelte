<script lang="ts">
	import type { AppError } from '#lib/api/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { errorOutput, refusedDirectivesReason } from '#lib/services/errors.js';
	import type { ProjectDraft } from '#lib/stores/projectDraft.svelte.js';

	/** `rejected` is the last save error: Apache refused the configuration, or Lampo refused a
	 *  directive that could run programs or expose the disk. */
	let { draft, rejected }: { draft: ProjectDraft; rejected: AppError | null } = $props();
	const t = m.projects.apache;
	const refusal = $derived(rejected ? refusedDirectivesReason(rejected) : null);
	const output = $derived(rejected?.code === 'config-rejected' ? errorOutput(rejected) : refusal);
	const invalid = $derived(rejected?.code === 'config-rejected' || refusal !== null);
</script>

<h3 class="text-sm font-medium">{t.title}</h3>
<p class="mb-2 text-xs text-muted-foreground">{t.help}</p>
<Textarea
	class="min-h-48 font-mono text-xs"
	spellcheck="false"
	placeholder={t.placeholder}
	bind:value={draft.environment.apacheDirectives}
	aria-label={t.title}
	aria-invalid={invalid}
/>
{#if invalid}
	<div class="mt-2 rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm" role="alert">
		<p class="font-medium text-destructive">{refusal !== null ? t.refused : t.rejected}</p>
		{#if output}<pre class="mt-1 max-h-48 overflow-auto font-mono text-xs whitespace-pre-wrap">{output}</pre>{/if}
	</div>
{/if}
