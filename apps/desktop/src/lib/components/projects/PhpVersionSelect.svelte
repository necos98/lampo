<script lang="ts">
	import Download from '@lucide/svelte/icons/download';

	import { Button } from '#lib/components/ui/button/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { describeError } from '#lib/services/errors.js';
	import { packages, tasks } from '#lib/stores/index.js';

	import SelectField from '../common/SelectField.svelte';
	import TaskProgress from '../common/TaskProgress.svelte';

	interface Props {
		/** PHP minor, or `null` for "use the default" (only with `allowDefault`). */
		value: string | null;
		allowDefault?: boolean;
		id?: string;
		'aria-describedby'?: string;
		onchange?: (value: string | null) => void;
	}

	let { value = $bindable(), allowDefault = false, id, 'aria-describedby': ariaDescribedBy, onchange }: Props = $props();

	const options = $derived([
		...(allowDefault ? [{ value: '', label: m.create.phpDefault }] : []),
		...packages.phpOptions.map((o) => ({
			value: o.series,
			label: `PHP ${o.series}${o.installed ? '' : ` · ${m.projects.php.notInstalled}`}${o.endOfLife ? ` · ${m.packages.endOfLife}` : ''}`
		}))
	]);
	// A project may point to a minor the catalog no longer lists: keep it selectable.
	const withCurrent = $derived(value && !options.some((o) => o.value === value) ? [...options, { value, label: `PHP ${value}` }] : options);
	const selected = $derived(packages.phpOptions.find((o) => o.series === value) ?? null);
	const missing = $derived(selected !== null && !selected.installed);
	const taskKey = $derived(value ? packages.taskKey('php', value, null) : '');
	const task = $derived(taskKey ? tasks.forKey(taskKey) : null);
	const installing = $derived(task !== null && task.outcome === null);
	/** Also true before the task is tracked: blocks a second click. */
	const busy = $derived(value !== null && packages.isInstalling({ kind: 'php', series: value, phpVersion: null }));
	const failure = $derived(task?.outcome?.status === 'failed' ? task.outcome.error : null);
</script>

<div class="flex flex-col gap-2">
	<div class="flex items-center gap-2">
		<SelectField {id} aria-describedby={ariaDescribedBy} value={value ?? ''} options={withCurrent} class="w-64" onchange={(v) => {
				value = v === '' ? null : v;
				onchange?.(value);
			}} />
		{#if missing && value && !installing}
			<Button size="sm" variant="secondary" disabled={busy} onclick={() => packages.install({ kind: 'php', series: value!, phpVersion: null })}>
				<Download />{m.projects.php.installNow}
			</Button>
		{/if}
	</div>
	{#if installing && task}
		<div class="max-w-sm"><TaskProgress progress={task.progress} oncancel={() => tasks.cancel(task.taskId)} /></div>
	{/if}
	{#if failure}<p class="text-xs text-destructive" role="alert">{m.packages.failed} {describeError(failure)}</p>{/if}
</div>
