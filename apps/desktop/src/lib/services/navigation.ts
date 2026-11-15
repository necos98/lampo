/** Top-level sections, in sidebar order. `Alt+<index+1>` jumps to each. */
export const NAV_ITEMS = [
	{ id: 'projects', href: '/projects' },
	{ id: 'services', href: '/services' },
	{ id: 'packages', href: '/packages' },
	{ id: 'logs', href: '/logs' },
	{ id: 'settings', href: '/settings' }
] as const;

export type NavId = (typeof NAV_ITEMS)[number]['id'];

/**
 * Keyboard shortcuts as a Svelte action on `window`-level handlers:
 * Alt+1..5 navigates, `/` focuses the first `[data-filter]` input, Escape blurs inputs.
 */
export function installShortcuts(navigate: (href: string) => void): () => void {
	const handler = (e: KeyboardEvent) => {
		const target = e.target as HTMLElement | null;
		const typing = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target?.isContentEditable === true;
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
