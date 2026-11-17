<script lang="ts">
	import { goto } from '$app/navigation';

	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { buildCommands, filterCommands, moveSelection, type PaletteActions, type PaletteCommand, type PaletteGroup } from '#lib/palette/commands.js';
	import { NAV_ITEMS, NEW_PROJECT_HREF } from '#lib/services/navigation.js';
	import { projects, services } from '#lib/stores/index.js';

	let { open = $bindable(false), onhelp }: { open: boolean; onhelp: () => void } = $props();

	const t = m.palette;
	const LIST_ID = 'command-palette-list';
	const optionId = (index: number) => `command-palette-option-${index}`;
	/** How far each arrow key moves the highlighted row. */
	const SELECTION_MOVES = new Map([
		['ArrowDown', 1],
		['ArrowUp', -1]
	]);

	/** The same operations as the pages; a project or service already being changed is left alone (a stop may still give up a start). */
	const actions: PaletteActions = {
		navigate: (href) => void goto(href),
		newProject: () => void goto(NEW_PROJECT_HREF),
		setProjectActive: (id, active) => {
			if (!projects.toggling[id]) void projects.setActive(id, active);
		},
		actOnServices: (action, names) => {
			if (services.canAct(action, names)) void services.act(action, names);
		},
		showHelp: () => onhelp()
	};

	let query = $state('');
	let selected = $state(0);
	let input = $state<HTMLInputElement | null>(null);

	const commands = $derived(buildCommands({ sections: NAV_ITEMS, projects: projects.all, services: services.list.data ?? [] }, actions, t));
	const results = $derived(filterCommands(commands, query));
	/** Kept in range when the results shrink under the cursor (a service changed state). */
	const active = $derived(Math.min(selected, Math.max(results.length - 1, 0)));
	/** The results under their group headings; `index` is the position in `results`. */
	const groups = $derived.by(() => {
		const byGroup = new Map<PaletteGroup, { command: PaletteCommand; index: number }[]>();
		results.forEach((command, index) => {
			const items = byGroup.get(command.group);
			if (items) items.push({ command, index });
			else byGroup.set(command.group, [{ command, index }]);
		});
		return [...byGroup].map(([group, items]) => ({ group, items }));
	});

	$effect(() => {
		if (open) document.getElementById(optionId(active))?.scrollIntoView({ block: 'nearest' });
	});

	// The dialog does not move the focus when it reopens while still closing (a shortcut right after
	// a command): the search field takes it explicitly, so the palette is always ready to type in.
	$effect(() => {
		if (open) input?.focus();
	});

	function run(command: PaletteCommand | undefined): void {
		if (!command) return;
		open = false;
		command.run();
	}

	function onkeydown(e: KeyboardEvent): void {
		const delta = SELECTION_MOVES.get(e.key);
		if (delta !== undefined) {
			e.preventDefault();
			selected = moveSelection(active, delta, results.length);
		} else if (e.key === 'Enter') {
			e.preventDefault();
			run(results[active]);
		}
	}

	/** Every opening starts from an empty search; a reopening during the close keeps what is typed. */
	function reset(isOpen: boolean): void {
		if (isOpen || open) return;
		query = '';
		selected = 0;
	}
</script>

<Dialog.Root bind:open onOpenChangeComplete={reset}>
	<Dialog.Content class="max-w-lg gap-0 overflow-hidden p-0" showCloseButton={false}>
		<Dialog.Title class="sr-only">{t.title}</Dialog.Title>
		<Dialog.Description class="sr-only">{t.description}</Dialog.Description>
		<input
			class="w-full border-b bg-transparent px-4 py-3 text-sm outline-none placeholder:text-muted-foreground"
			type="text"
			role="combobox"
			aria-expanded="true"
			aria-controls={LIST_ID}
			aria-activedescendant={results.length ? optionId(active) : undefined}
			aria-autocomplete="list"
			aria-label={t.title}
			placeholder={t.placeholder}
			bind:this={input}
			autocomplete="off"
			spellcheck="false"
			bind:value={() => query, (value) => ((query = value), (selected = 0))}
			{onkeydown}
		/>
		<div id={LIST_ID} role="listbox" aria-label={t.title} class="max-h-80 overflow-y-auto p-1">
			{#each groups as { group, items } (group)}
				<div role="group" aria-labelledby="command-palette-group-{group}">
					<p id="command-palette-group-{group}" class="px-3 pt-2 pb-1 text-xs font-medium text-muted-foreground">{t.groups[group]}</p>
					{#each items as { command, index } (command.id)}
						<!-- Driven from the search field (aria-activedescendant): options never take focus. -->
						<!-- svelte-ignore a11y_click_events_have_key_events -->
						<div
							id={optionId(index)}
							role="option"
							tabindex="-1"
							aria-selected={index === active}
							class="cursor-pointer rounded-md px-3 py-1.5 text-sm {index === active ? 'bg-accent text-accent-foreground' : ''}"
							onpointermove={() => (selected = index)}
							onclick={() => run(command)}
						>
							{command.label}
						</div>
					{/each}
				</div>
			{:else}
				<p class="px-3 py-6 text-center text-sm text-muted-foreground">{t.empty}</p>
			{/each}
		</div>
	</Dialog.Content>
</Dialog.Root>
