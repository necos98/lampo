<script lang="ts">
	import FolderOpen from '@lucide/svelte/icons/folder-open';

	import type { FileFilter } from '#lib/backend/ports.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { pickFile, pickFolder, pickSaveFile } from '#lib/services/platform.js';

	interface Props {
		value: string;
		/** `folder`: an existing folder; `file`: an existing file; `save`: a file to write. */
		mode: 'folder' | 'file' | 'save';
		id?: string;
		'aria-describedby'?: string;
		placeholder?: string;
		required?: boolean;
		/** Title of the native picker. */
		title?: string;
		filters?: FileFilter[];
		autofocus?: boolean;
		oninput?: () => void;
		onblur?: () => void;
		/** Called after a path was picked (e.g. to start detection). */
		onpicked?: (path: string) => void;
	}

	let { value = $bindable(), mode, id, 'aria-describedby': ariaDescribedBy, placeholder = '', required = false, title, filters, autofocus = false, oninput, onblur, onpicked }: Props = $props();

	async function browse() {
		const defaultPath = value.trim() || undefined;
		const pick = mode === 'folder' ? pickFolder : mode === 'file' ? pickFile : pickSaveFile;
		const picked = await pick({ title, defaultPath, filters });
		if (!picked) return;
		value = picked;
		oninput?.();
		onpicked?.(picked);
	}
</script>

<div class="flex gap-2">
	<!-- svelte-ignore a11y_autofocus -->
	<Input {id} aria-describedby={ariaDescribedBy} class="h-8 font-mono text-xs" bind:value {placeholder} {required} {autofocus} {oninput} {onblur} />
	<Button type="button" size="sm" variant="outline" onclick={browse} aria-label={title ?? m.common.browse}><FolderOpen />{m.common.browse}</Button>
</div>
