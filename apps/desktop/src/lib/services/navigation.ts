/** Top-level sections, in sidebar order. `Alt+<index+1>` jumps to each. */
export const NAV_ITEMS = [
	{ id: 'projects', href: '/projects' },
	{ id: 'services', href: '/services' },
	{ id: 'packages', href: '/packages' },
	{ id: 'logs', href: '/logs' },
	{ id: 'settings', href: '/settings' }
] as const;

export type NavId = (typeof NAV_ITEMS)[number]['id'];

export interface ShortcutActions {
	navigate: (href: string) => void;
	/** Saves the form on screen, if any has unsaved edits. */
	save: () => void;
}

type ShortcutKey = Pick<KeyboardEvent, 'key' | 'altKey' | 'ctrlKey' | 'metaKey' | 'shiftKey'>;

/** Ctrl+S (Cmd+S), whatever the keyboard layout reports as the letter's case. */
export function isSaveShortcut(e: ShortcutKey): boolean {
	return (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 's';
}

/**
 * Keyboard shortcuts as a Svelte action on `window`-level handlers: Alt+1..5 navigates, Ctrl+S
 * saves, `/` focuses the first `[data-filter]` input, Escape blurs inputs.
 */
export function installShortcuts({ navigate, save }: ShortcutActions): () => void {
	const handler = (e: KeyboardEvent) => {
		const target = e.target as HTMLElement | null;
		const typing = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target?.isContentEditable === true;
		if (isSaveShortcut(e)) {
			// Always swallowed: the webview would otherwise offer to save the page as HTML.
			e.preventDefault();
			if (!e.repeat) save();
			return;
		}
		if (e.altKey && !e.ctrlKey && !e.metaKey) {
			const index = Number(e.key) - 1;
			if (Number.isInteger(index) && NAV_ITEMS[index]) {
				e.preventDefault();
				navigate(NAV_ITEMS[index].href);
			}
			return;
		}
		if (e.key === '/' && !typing) {
			const input = document.querySelector<HTMLInputElement>('[data-filter]');
			if (input) {
				e.preventDefault();
				input.focus();
				input.select();
			}
		} else if (e.key === 'Escape' && typing && target?.hasAttribute('data-filter')) {
			(target as HTMLInputElement).blur();
		}
	};
	window.addEventListener('keydown', handler);
	return () => window.removeEventListener('keydown', handler);
}
