/** A form with an editable draft that must not be lost silently (navigation, Ctrl+S). */
export interface UnsavedForm {
	readonly dirty: boolean;
	/** Saves the draft; a form that cannot save right now (invalid, already saving) does nothing. */
	save(): Promise<void>;
	/** Drops the edits, back to the last saved value. */
	discard(): void;
}

/**
 * The forms currently on screen. The shell asks it whether leaving would lose edits and routes the
 * save shortcut to it, without knowing which page is open.
 */
export class UnsavedChanges {
	readonly #forms = new Set<UnsavedForm>();

	/** Adds `form` while it is mounted; call the returned function on unmount. */
	register(form: UnsavedForm): () => void {
		this.#forms.add(form);
		return () => this.#forms.delete(form);
	}

	get dirty(): boolean {
		return this.#dirtyForms().length > 0;
	}

	async saveAll(): Promise<void> {
		await Promise.all(this.#dirtyForms().map((form) => form.save()));
	}

	discardAll(): void {
		for (const form of this.#dirtyForms()) form.discard();
	}

	#dirtyForms(): UnsavedForm[] {
		return [...this.#forms].filter((form) => form.dirty);
	}
}
