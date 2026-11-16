<script lang="ts">
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';
	import { tick } from 'svelte';

	import type { AppError } from '#lib/api/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { problemPart, sameEnvVarName, type EnvVarPart, type EnvVarRow } from '#lib/projects/envVarList.js';
	import { envVarRefusal, type EnvVarRefusal } from '#lib/projects/envVarRefusal.js';
	import type { ProjectDraft } from '#lib/stores/projectDraft.svelte.js';

	/** `rejected` is the last save error: the backend's refusal is shown on the row it names. */
	let { draft, rejected }: { draft: ProjectDraft; rejected: AppError | null } = $props();
	const list = $derived(draft.envVars);
	const t = m.projects.envVars;
	const problems = $derived(list.problems);

	const refusal = $derived(rejected ? envVarRefusal(rejected) : null);
	/** The row the refusal names, while it still holds that name. */
	const refusedRowId = $derived.by(() => {
		const name = refusal?.name;
		return name == null ? null : (list.rows.find((row) => sameEnvVarName(row.name, name))?.id ?? null);
	});
	// A refusal naming a variable shows on its row, and goes with it once renamed or removed.
	const listRefusal = $derived(refusal !== null && refusal.name === null ? refusal : null);

	interface RowError {
		part: EnvVarPart;
		text: string;
	}

	function refusalText(r: EnvVarRefusal): string {
		if (r.reserved) return t.reserved;
		return r.part === 'value' ? t.valueRefused(r.reason) : t.nameRefused(r.reason);
	}

	/** The client-side problem first (it is the latest), else the backend's refusal of this row. */
	function rowError(row: EnvVarRow): RowError | null {
		const problem = problems.get(row.id);
		if (problem) return { part: problemPart(problem), text: t.problems[problem] };
		if (refusal !== null && row.id === refusedRowId) return { part: refusal.part ?? 'name', text: refusalText(refusal) };
		return null;
	}

	async function add() {
		const id = list.add();
		await tick();
		document.getElementById(nameInputId(id))?.focus();
	}

	const nameInputId = (id: number) => `env-var-${id}-name`;
	const errorId = (id: number) => `env-var-${id}-error`;
</script>

<!-- Above the list: a row added below would move the button out from under a double click. -->
<div class="mb-3 flex items-start justify-between gap-4">
	<p class="text-sm text-muted-foreground">{t.help}</p>
	<Button variant="secondary" size="sm" class="shrink-0" onclick={add}><Plus />{t.add}</Button>
</div>
<ul class="flex flex-col gap-2">
	{#each list.rows as row, i (row.id)}
		{@const error = rowError(row)}
		<li class="flex flex-col gap-1">
			<div class="flex items-center gap-2">
				<Input
					id={nameInputId(row.id)}
					class="h-8 w-64 font-mono text-xs"
					placeholder="APP_ENV"
					spellcheck="false"
					autocomplete="off"
					value={row.name}
					oninput={(e) => list.setName(row.id, e.currentTarget.value)}
					aria-label={t.name}
					aria-invalid={error?.part === 'name'}
					aria-describedby={error?.part === 'name' ? errorId(row.id) : undefined}
				/>
				<span class="font-mono text-xs text-muted-foreground" aria-hidden="true">=</span>
				<Input
					class="h-8 flex-1 font-mono text-xs"
					spellcheck="false"
					autocomplete="off"
					value={row.value}
					oninput={(e) => list.setValue(row.id, e.currentTarget.value)}
					aria-label={t.value}
					aria-invalid={error?.part === 'value'}
					aria-describedby={error?.part === 'value' ? errorId(row.id) : undefined}
				/>
				<Button variant="ghost" size="icon-xs" onclick={() => list.remove(row.id)} aria-label="{m.common.remove} {row.name.trim() || i + 1}"><X /></Button>
			</div>
			{#if error}<p id={errorId(row.id)} class="text-xs text-destructive">{error.text}</p>{/if}
		</li>
	{/each}
</ul>
{#if list.rows.length === 0}<p class="text-sm text-muted-foreground">{t.empty}</p>{/if}
{#if listRefusal}
	<p class="mt-2 rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive" role="alert">{t.refused(listRefusal.reason)}</p>
{/if}
