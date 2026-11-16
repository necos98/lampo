<script lang="ts">
	import FolderSearch from '@lucide/svelte/icons/folder-search';

	import ErrorNotice from '#lib/components/common/ErrorNotice.svelte';
	import FormRow from '#lib/components/common/FormRow.svelte';
	import PathInput from '#lib/components/common/PathInput.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import type { CreateProjectFlow } from '#lib/stores/createProject.svelte.js';

	let { flow }: { flow: CreateProjectFlow } = $props();
	const t = m.create;
</script>

<Card.Root size="sm">
	<Card.Header><Card.Title>{t.stepFolder}</Card.Title></Card.Header>
	<Card.Content class="flex flex-col gap-2">
		<FormRow label={t.folderLabel} help={t.folderHelp}>
			{#snippet children(field)}
				<div class="flex gap-2">
					<div class="flex-1">
						<PathInput
							{...field}
							mode="folder"
							title={t.folderLabel}
							placeholder={t.folderPlaceholder}
							bind:value={flow.folder}
							oninput={() => flow.folderEdited()}
							onblur={() => flow.folder.trim() && !flow.detection && void flow.detect()}
							onpicked={() => void flow.detect()}
							autofocus
						/>
					</div>
					<Button type="button" size="sm" variant="secondary" onclick={() => flow.detect()} disabled={!flow.folder.trim() || flow.detecting}>
						<FolderSearch />{flow.detecting ? t.detecting : t.detect}
					</Button>
				</div>
			{/snippet}
		</FormRow>
		{#if flow.detectError}<ErrorNotice error={flow.detectError} title={m.common.loadFailed} />{/if}
	</Card.Content>
</Card.Root>
