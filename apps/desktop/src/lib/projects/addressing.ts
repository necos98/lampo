import type { Addressing } from '../api/types.js';

/**
 * Whether each addressing mode writes a name into the Windows hosts file, which needs the
 * administrator's permission (the elevation flow). A `Record` so a new backend mode fails to
 * compile until it is listed here.
 */
const WRITES_HOSTS_FILE: Record<Addressing, boolean> = {
	'no-elevation': false,
	standard: true,
	'hosts-only': true
};

/** Every mode, in display order: the recommended one (no administrator rights) first. */
export const ADDRESSING_MODES = Object.keys(WRITES_HOSTS_FILE) as Addressing[];

/** The built-in default (as the backend's), until the settings say otherwise. */
export const FALLBACK_ADDRESSING: Addressing = 'no-elevation';

export function needsElevation(mode: Addressing): boolean {
	return WRITES_HOSTS_FILE[mode];
}

/** The mode a new project gets: the user's choice, else the global default, else the built-in one. */
export function effectiveAddressing(chosen: Addressing | null, configuredDefault: Addressing | null | undefined): Addressing {
	return chosen ?? configuredDefault ?? FALLBACK_ADDRESSING;
}
