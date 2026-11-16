<script lang="ts">
	import type { StarterKit, TestingFramework } from '#lib/api/index.js';

	import FormRow from '#lib/components/common/FormRow.svelte';
	import SelectField from '#lib/components/common/SelectField.svelte';
	import PhpVersionSelect from '#lib/components/projects/PhpVersionSelect.svelte';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { packages } from '#lib/stores/index.js';
	import { LARAVEL_MIN_PHP, compareMinor, type CreateProjectFlow } from '#lib/stores/createProject.svelte.js';

	import ComposerNotice from './ComposerNotice.svelte';

	let { flow }: { flow: CreateProjectFlow } = $props();
	const t = m.create;

	const starterKitOptions = (Object.keys(t.starterKits) as StarterKit[]).map((k) => ({ value: k, label: t.starterKits[k] }));
	const testingOptions = (Object.keys(t.testingFrameworks) as TestingFramework[]).map((k) => ({ value: k, label: t.testingFrameworks[k] }));

	// Laravel: newest installed PHP the installer supports, until the user picks one.
	const newestLaravelPhp = $derived(
		packages.phpOptions
			.filter((o) => o.installed && compareMinor(o.series, LARAVEL_MIN_PHP) >= 0)
			.map((o) => o.series)
			.sort(compareMinor)
			.at(-1) ?? null
	);
	$effect(() => {
		if (flow.source === 'laravel' && flow.laravelPhp === null && newestLaravelPhp) flow.laravelPhp = newestLaravelPhp;
	});
</script>

<FormRow label={t.laravelPhp} help={t.laravelPhpHelp}>
	{#snippet children(field)}
		<PhpVersionSelect {...field} bind:value={flow.laravelPhp} />
		{#if !flow.laravelPhpSupported}<p class="mt-1 text-xs text-destructive">{t.laravelPhpTooOld}</p>{/if}
	{/snippet}
</FormRow>
<FormRow label={t.starterKit}>
	{#snippet children(field)}
		<SelectField {...field} value={flow.starterKit} options={starterKitOptions} class="w-48" onchange={(v) => (flow.starterKit = v as StarterKit)} />
	{/snippet}
</FormRow>
<FormRow label={t.testing}>
	{#snippet children(field)}
		<SelectField {...field} value={flow.testing} options={testingOptions} class="w-48" onchange={(v) => (flow.testing = v as TestingFramework)} />
	{/snippet}
</FormRow>
<FormRow label={t.initGit} help={t.initGitHelp}>
	{#snippet children(field)}
		<Switch {...field} bind:checked={flow.initGit} />
	{/snippet}
</FormRow>
<FormRow label={t.npm} help={t.npmHelp}>
	{#snippet children(field)}
		<Switch {...field} bind:checked={flow.npm} />
	{/snippet}
</FormRow>
<ComposerNotice />
