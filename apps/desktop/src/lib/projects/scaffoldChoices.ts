import type { StarterKit, TestingFramework } from '../api/index.js';

/**
 * What the "new project" form remembers from the last successful creation. The addressing mode
 * is deliberately not here: its single source stays the `default_addressing` setting.
 */
export interface ScaffoldChoices {
	/** `''` when nothing is remembered. */
	parentDir: string;
	starterKit: StarterKit;
	testing: TestingFramework;
}

// `Record` keeps the lists complete: a value added to the contract fails type-checking here.
const STARTER_KIT_SET: Record<StarterKit, true> = { none: true, react: true, vue: true, livewire: true, svelte: true };
const TESTING_FRAMEWORK_SET: Record<TestingFramework, true> = { pest: true, phpunit: true };
export const STARTER_KITS = Object.keys(STARTER_KIT_SET) as readonly StarterKit[];
export const TESTING_FRAMEWORKS = Object.keys(TESTING_FRAMEWORK_SET) as readonly TestingFramework[];
export const DEFAULT_SCAFFOLD_CHOICES: Readonly<ScaffoldChoices> = { parentDir: '', starterKit: 'none', testing: 'pest' };

/** `localStorage` key of the remembered choices. */
export const SCAFFOLD_CHOICES_KEY = 'lampo.projects.scaffoldChoices';

type ChoicesStorage = Pick<Storage, 'getItem' | 'setItem'>;

/** `localStorage`, or `null` where it does not exist; even reaching it can throw (blocked site data). */
const browserStorage = (): ChoicesStorage | null => (typeof localStorage === 'undefined' ? null : localStorage);

const isOneOf = <V extends string>(values: readonly V[], value: unknown): value is V => typeof value === 'string' && (values as readonly string[]).includes(value);

/** The stored choices, field by field: anything missing, unknown or malformed falls back to the default. */
export function parseScaffoldChoices(raw: string | null): ScaffoldChoices {
	let stored: unknown = null;
	try {
		stored = raw === null ? null : JSON.parse(raw);
	} catch {
		// Edited or corrupted by hand: the defaults apply.
	}
	const fields = typeof stored === 'object' && stored !== null ? (stored as Record<string, unknown>) : {};
	return {
		parentDir: typeof fields.parentDir === 'string' ? fields.parentDir.trim() : DEFAULT_SCAFFOLD_CHOICES.parentDir,
		starterKit: isOneOf(STARTER_KITS, fields.starterKit) ? fields.starterKit : DEFAULT_SCAFFOLD_CHOICES.starterKit,
		testing: isOneOf(TESTING_FRAMEWORKS, fields.testing) ? fields.testing : DEFAULT_SCAFFOLD_CHOICES.testing
	};
}

/** The remembered choices, or the defaults when storage is unavailable or refuses the read. */
export function loadScaffoldChoices(storage: () => ChoicesStorage | null = browserStorage): ScaffoldChoices {
	try {
		return parseScaffoldChoices(storage()?.getItem(SCAFFOLD_CHOICES_KEY) ?? null);
	} catch {
		return { ...DEFAULT_SCAFFOLD_CHOICES };
	}
}

/**
 * Remembers `choices` over the stored ones (fields left out keep their remembered value);
 * `false` when storage is unavailable or refuses the write. Losing them only loses the convenience.
 */
export function rememberScaffoldChoices(choices: Partial<ScaffoldChoices>, storage: () => ChoicesStorage | null = browserStorage): boolean {
	try {
		const target = storage();
		if (!target) return false;
		const next = { ...parseScaffoldChoices(target.getItem(SCAFFOLD_CHOICES_KEY)), ...choices };
		target.setItem(SCAFFOLD_CHOICES_KEY, JSON.stringify({ parentDir: next.parentDir.trim(), starterKit: next.starterKit, testing: next.testing }));
		return true;
	} catch {
		return false;
	}
}
