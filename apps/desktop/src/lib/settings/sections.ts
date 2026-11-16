/**
 * The sections of the Settings page, in display order. A section either edits the saved settings
 * (its fields go into one draft, applied by the SaveBar) or acts at once (buttons that run an
 * operation, read-only information): the page groups and labels them so the difference is visible.
 */
export const SETTINGS_SECTION_KINDS = ['saved', 'immediate'] as const;
export type SettingsSectionKind = (typeof SETTINGS_SECTION_KINDS)[number];

export const SETTINGS_SECTIONS = [
	{ id: 'general', kind: 'saved' },
	{ id: 'ports', kind: 'saved' },
	{ id: 'updates', kind: 'immediate' },
	{ id: 'tools', kind: 'immediate' },
	{ id: 'presets', kind: 'immediate' },
	{ id: 'doctor', kind: 'immediate' },
	{ id: 'about', kind: 'immediate' }
] as const satisfies readonly { id: string; kind: SettingsSectionKind }[];

export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];
export type SettingsSectionId = SettingsSection['id'];

/** The section shown when the page opens. */
export const DEFAULT_SETTINGS_SECTION: SettingsSectionId = SETTINGS_SECTIONS[0].id;

/** Sections of `kind`, in display order. */
export function sectionsOfKind(kind: SettingsSectionKind): SettingsSection[] {
	return SETTINGS_SECTIONS.filter((section) => section.kind === kind);
}

/** Whether `id` (e.g. the current tab value) names a section whose fields wait for Save. */
export function isSavedSection(id: string): boolean {
	return SETTINGS_SECTIONS.some((section) => section.id === id && section.kind === 'saved');
}

/** The section with `id`. */
export function settingsSection(id: SettingsSectionId): SettingsSection {
	// The ids come from SETTINGS_SECTIONS itself, so the lookup always succeeds.
	return SETTINGS_SECTIONS.find((section) => section.id === id)!;
}
