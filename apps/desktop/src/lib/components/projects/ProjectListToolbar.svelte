<script lang="ts">
	import SelectField from '#lib/components/common/SelectField.svelte';
	import { Input } from '#lib/components/ui/input/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { PROJECT_SORTS, PROJECT_STATUS_FILTERS, type ProjectView } from '#lib/projects/projectView.js';

	import BulkActivationButtons from './BulkActivationButtons.svelte';

	interface Props {
		/** Text filter (name, id, folder, URL). */
		query: string;
		/** Order and status filter; the page remembers it. */
		view: ProjectView;
	}

	let { query = $bindable(), view = $bindable() }: Props = $props();
	const t = m.projects.view;

	const sortOptions = PROJECT_SORTS.map((value) => ({ value, label: t.sorts[value] }));
	const statusOptions = PROJECT_STATUS_FILTERS.map((value) => ({ value, label: t.statuses[value] }));

	// The options come from the same lists, so a value not in them cannot arrive: it would be ignored.
	const setSort = (value: string) => (view.sort = PROJECT_SORTS.find((s) => s === value) ?? view.sort);
	const setStatus = (value: string) => (view.status = PROJECT_STATUS_FILTERS.find((s) => s === value) ?? view.status);
</script>

<div class="mb-3 flex flex-wrap items-center gap-2">
	<Input data-filter class="h-8 max-w-xs" placeholder={m.common.search} bind:value={query} aria-label={m.common.search} />
	<SelectField value={view.status} options={statusOptions} class="w-44" aria-label={t.status} onchange={setStatus} />
	<SelectField value={view.sort} options={sortOptions} class="w-40" aria-label={t.sort} onchange={setSort} />
	<div class="ml-auto flex items-center gap-2">
		<BulkActivationButtons />
	</div>
</div>
