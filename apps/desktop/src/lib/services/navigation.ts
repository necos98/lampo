/** Top-level sections, in sidebar order. `Alt+<index+1>` jumps to each. */
export const NAV_ITEMS = [
	{ id: 'projects', href: '/projects' },
	{ id: 'services', href: '/services' },
	{ id: 'packages', href: '/packages' },
	{ id: 'logs', href: '/logs' },
	{ id: 'settings', href: '/settings' }
] as const;

export type NavId = (typeof NAV_ITEMS)[number]['id'];

/** The create screen of a new project (Ctrl+N). */
export const NEW_PROJECT_HREF = '/projects/new';

/** Marks the list filter `/` focuses (the first one on the page). */
const FILTER_ATTRIBUTE = 'data-filter';
/** Marks the search field Ctrl+F focuses, where a page has one (the log viewer). */
const FIND_ATTRIBUTE = 'data-find';

export interface ShortcutActions {
	/** In-app navigation: it goes through the unsaved-changes guard like any other. */
	navigate: (href: string) => void;
	/** Saves the form on screen, if any has unsaved edits. */
	save: () => void;
	/** Opens the command palette, or closes it when it is open. */
	togglePalette: () => void;
	/** Opens the list of the shortcuts. */
	showHelp: () => void;
}

/** What a key press asks for, independent of the page it happens on. */
export type Shortcut =
	| { kind: 'navigate'; href: string }
	| { kind: 'save' }
	| { kind: 'palette' }
	| { kind: 'help' }
	| { kind: 'find' }
	| { kind: 'focusFilter' }
	| { kind: 'leaveFilter' };

type ShortcutKey = Pick<KeyboardEvent, 'key' | 'altKey' | 'ctrlKey' | 'metaKey' | 'shiftKey'>;

/** Where the key press happened: in a text field (where letters are typing), in a list filter. */
export interface KeyContext {
	typing: boolean;
	inFilter: boolean;
}

/** Ctrl+<letter> (Cmd on macOS), whatever the keyboard layout reports as the letter's case. */
function isCtrlLetter(e: ShortcutKey, letter: string): boolean {
	return (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === letter;
}

/** Ctrl+S (Cmd+S), whatever the keyboard layout reports as the letter's case. */
export function isSaveShortcut(e: ShortcutKey): boolean {
	return isCtrlLetter(e, 's');
}

const CTRL_SHORTCUTS: Readonly<Record<string, Shortcut>> = {
	s: { kind: 'save' },
	n: { kind: 'navigate', href: NEW_PROJECT_HREF },
	k: { kind: 'palette' },
	f: { kind: 'find' }
};

/**
 * The shortcut `e` stands for, or `null` for an ordinary key press. Ctrl shortcuts work while
 * typing; single keys (`/`, `?`) only outside text fields, where they would otherwise be typed.
 */
export function matchShortcut(e: ShortcutKey, { typing, inFilter }: KeyContext): Shortcut | null {
	for (const [letter, shortcut] of Object.entries(CTRL_SHORTCUTS)) {
		if (isCtrlLetter(e, letter)) return shortcut;
	}
	if (e.altKey && !e.ctrlKey && !e.metaKey) {
		const item = NAV_ITEMS[Number(e.key) - 1];
		return item ? { kind: 'navigate', href: item.href } : null;
	}
	if (e.ctrlKey || e.metaKey) return null;
	if (e.key === 'Escape') return typing && inFilter ? { kind: 'leaveFilter' } : null;
	if (typing) return null;
	if (e.key === '/') return { kind: 'focusFilter' };
	if (e.key === '?') return { kind: 'help' };
	return null;
}

/** Focuses and selects the first field matching `selector`; `false` when the page has none. */
function focusField(selector: string): boolean {
	const input = document.querySelector<HTMLInputElement>(selector);
	if (!input || input.disabled) return false;
	input.focus();
	input.select();
	return true;
}

function isTextField(target: EventTarget | null): target is HTMLElement {
	return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || (target instanceof HTMLElement && target.isContentEditable);
}

/**
 * Installs the window-level shortcuts (see `matchShortcut`) and returns their disposer. A shortcut
 * the page cannot serve (Ctrl+F without a search field) keeps the key's default behaviour.
 */
export function installShortcuts(actions: ShortcutActions): () => void {
	/** Runs `shortcut`; `true` when it was served, so the key's default behaviour is prevented. */
	const run = (shortcut: Shortcut, field: HTMLElement | null, repeat: boolean): boolean => {
		switch (shortcut.kind) {
			case 'save':
				if (!repeat) actions.save();
				// Always swallowed: the webview would otherwise offer to save the page as HTML.
				return true;
			case 'navigate':
				if (!repeat) actions.navigate(shortcut.href);
				// Always swallowed: Ctrl+N would otherwise open a second webview window.
				return true;
			case 'palette':
				if (!repeat) actions.togglePalette();
				return true;
			case 'help':
				actions.showHelp();
				return true;
			case 'find':
				return focusField(`[${FIND_ATTRIBUTE}]`);
			case 'focusFilter':
				return focusField(`[${FILTER_ATTRIBUTE}]`);
			case 'leaveFilter':
				field?.blur();
				return false;
		}
	};
	const handler = (e: KeyboardEvent) => {
		// Autofill and some IMEs dispatch keydown events without a key.
		if (typeof e.key !== 'string') return;
		const field = isTextField(e.target) ? e.target : null;
		const shortcut = matchShortcut(e, { typing: field !== null, inFilter: field?.hasAttribute(FILTER_ATTRIBUTE) === true });
		if (shortcut && run(shortcut, field, e.repeat)) e.preventDefault();
	};
	window.addEventListener('keydown', handler);
	return () => window.removeEventListener('keydown', handler);
}

/** An entry of the shortcuts help: the keys to press and what they do (`messages.shortcuts.help`). */
export const SHORTCUT_HELP = [
	{ id: 'palette', keys: ['Ctrl', 'K'] },
	{ id: 'newProject', keys: ['Ctrl', 'N'] },
	{ id: 'save', keys: ['Ctrl', 'S'] },
	{ id: 'sections', keys: ['Alt', '1…5'] },
	{ id: 'find', keys: ['Ctrl', 'F'] },
	{ id: 'filter', keys: ['/'] },
	{ id: 'leave', keys: ['Esc'] },
	{ id: 'help', keys: ['?'] }
] as const;

export type ShortcutHelpId = (typeof SHORTCUT_HELP)[number]['id'];
