/**
 * The confirmation of «Attiva tutti» / «Disattiva tutti». The number it states is taken when it
 * opens: once confirmed, the projects being switched leave the live count (they are toggling), and
 * the dialog, still open while the switch runs, must not turn into «0 progetti».
 */
export class BulkConfirmation {
	open = $state(false);
	/** Serve every project (`true`) or stop every one. */
	target = $state(true);
	/** How many projects the open confirmation applies to. */
	count = $state(0);

	readonly #countOf: (active: boolean) => number;

	/** `countOf` is the live number of projects a switch to `active` would change. */
	constructor(countOf: (active: boolean) => number) {
		this.#countOf = countOf;
	}

	ask(active: boolean): void {
		this.target = active;
		this.count = this.#countOf(active);
		this.open = true;
	}
}
