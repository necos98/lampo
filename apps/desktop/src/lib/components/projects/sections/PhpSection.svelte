<script lang="ts">
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';

	import FormRow from '#lib/components/common/FormRow.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Checkbox } from '#lib/components/ui/checkbox/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { extensionChoices } from '#lib/projects/phpExtensions.js';
	import { packages } from '#lib/stores/index.js';
	import type { ProjectDraft } from '#lib/stores/projectDraft.svelte.js';

	import PhpVersionSelect from '../PhpVersionSelect.svelte';

	let { draft }: { draft: ProjectDraft } = $props();
	const t = m.projects.php;

	// What the selected PHP minor actually ships (common list until known / if not installed).
	$effect(() => packages.loadExtensions(draft.environment.phpVersion));
	const available = $derived(packages.extensionsOf(draft.environment.phpVersion));
	const choices = $derived(extensionChoices(draft.environment.extensions, available));
	// Add-on kinds come from the catalog (the backend groups PHP-pinned packages as `addons`).
	$effect(() => {
		if (packages.catalog.data === null) void packages.loadLocal();
	});
	const addons = $derived([...new Set([...packages.rowsOfGroup('addons').map((r) => r.kind), ...draft.environment.addons])].sort());
	// Rows of the ini editor keep their position while the key is edited.
	const iniRows = $derived(Object.entries(draft.environment.phpIni));
	let newKey = $state('');
	let newValue = $state('');

	function toggle(list: 'extensions' | 'addons', name: string, on: boolean) {
		const current = draft.environment[list];
		draft.environment[list] = on ? [...new Set([...current, name])] : current.filter((e) => e !== name);
	}

	function addIni() {
		const key = newKey.trim();
		if (!key) return;
		draft.environment.phpIni = { ...draft.environment.phpIni, [key]: newValue };
		newKey = newValue = '';
	}

	function removeIni(key: string) {
		const { [key]: _removed, ...rest } = draft.environment.phpIni;
		draft.environment.phpIni = rest;
	}

	function setIni(key: string, value: string) {
		draft.environment.phpIni = { ...draft.environment.phpIni, [key]: value };
	}
</script>

<FormRow label={t.version} for="php-version">
	<PhpVersionSelect id="php-version" value={draft.environment.phpVersion} onchange={(v) => v && (draft.environment.phpVersion = v)} />
</FormRow>

<FormRow label={t.extensions} help={available ? t.extensionsOfBuild(draft.environment.phpVersion) : t.extensionsCommon}>
	<ul class="grid grid-cols-2 gap-x-4 gap-y-1.5 sm:grid-cols-3 lg:grid-cols-4">
		{#each choices as ext (ext)}
			<li class="flex items-center gap-2">
				<Checkbox id="ext-{ext}" checked={draft.environment.extensions.includes(ext)} onCheckedChange={(v) => toggle('extensions', ext, v === true)} />
				<Label for="ext-{ext}" class="font-mono text-xs font-normal">{ext}</Label>
			</li>
		{/each}
	</ul>
</FormRow>

<FormRow label={t.addons}>
	<ul class="flex flex-wrap gap-x-6 gap-y-1.5">
		{#each addons as addon (addon)}
			<li class="flex items-center gap-2">
				<Checkbox id="addon-{addon}" checked={draft.environment.addons.includes(addon)} onCheckedChange={(v) => toggle('addons', addon, v === true)} />
				<Label for="addon-{addon}" class="text-sm font-normal">{t.addonLabels[addon] ?? addon}</Label>
			</li>
		{/each}
	</ul>
</FormRow>

<FormRow label={t.ini} help={t.iniHelp}>
	<ul class="flex flex-col gap-1.5">
		{#each iniRows as [key, value] (key)}
			<li class="flex items-center gap-2">
				<code class="w-56 truncate rounded bg-muted px-2 py-1 font-mono text-xs">{key}</code>
				<Input class="h-8 max-w-48 font-mono text-xs" {value} oninput={(e) => setIni(key, e.currentTarget.value)} aria-label={key} />
				<Button variant="ghost" size="icon-xs" onclick={() => removeIni(key)} aria-label="{m.common.remove} {key}"><X /></Button>
			</li>
		{/each}
		<li class="flex items-center gap-2">
			<Input class="h-8 w-56 font-mono text-xs" placeholder={t.iniKey} bind:value={newKey} onkeydown={(e) => e.key === 'Enter' && (e.preventDefault(), addIni())} aria-label={t.iniKey} />
			<Input class="h-8 max-w-48 font-mono text-xs" placeholder={t.iniValue} bind:value={newValue} onkeydown={(e) => e.key === 'Enter' && (e.preventDefault(), addIni())} aria-label={t.iniValue} />
			<Button variant="secondary" size="sm" onclick={addIni} disabled={!newKey.trim()}><Plus />{t.iniAdd}</Button>
		</li>
	</ul>
</FormRow>
