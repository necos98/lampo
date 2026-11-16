import type { AddressSuggestions, SiteAddress } from '../api/index.js';

export type AddressKind = SiteAddress['kind'];

/** One row of the addresses editor; `id` stays the same while rows are edited or removed. */
export interface AddressRow {
	readonly id: number;
	address: SiteAddress;
}

/** Fresh suggestions from the backend (domain rules + port allocator); null when unavailable. */
export type SuggestAddresses = () => Promise<AddressSuggestions | null>;

/** The empty address of `kind`: what a row shows until a suggestion fills it, or the user does. */
function emptyAddress(kind: AddressKind): SiteAddress {
	return kind === 'loopback-port' ? { kind, port: NaN } : { kind, host: '' };
}

/**
 * The addresses of a project draft, edited by row id. A new row (or a row changing kind) appears
 * at once as an empty address and is filled when its suggestion arrives, so quick clicks never
 * act on a stale index. `rows` stores the rows; pass a `$state` array so the UI is reactive.
 */
export class AddressList {
	readonly #rows: AddressRow[];
	/** Rows waiting for a suggestion, with the request they wait for: any later edit drops it. */
	readonly #pending = new Map<number, symbol>();
	#nextId = 0;

	constructor(rows: AddressRow[] = []) {
		this.#rows = rows;
	}

	get rows(): readonly AddressRow[] {
		return this.#rows;
	}

	get values(): SiteAddress[] {
		return this.#rows.map((row) => row.address);
	}

	/** Replaces every row with `addresses`, dropping the suggestions still on their way. */
	load(addresses: SiteAddress[]): void {
		this.#pending.clear();
		this.#rows.splice(0, this.#rows.length, ...addresses.map((address) => ({ id: this.#nextId++, address })));
	}

	add(kind: AddressKind, suggest: SuggestAddresses): Promise<void> {
		const id = this.#nextId++;
		this.#rows.push({ id, address: emptyAddress(kind) });
		return this.#fill(id, kind, suggest);
	}

	async changeKind(id: number, kind: AddressKind, suggest: SuggestAddresses): Promise<void> {
		const row = this.#row(id);
		if (!row || row.address.kind === kind) return;
		row.address = emptyAddress(kind);
		await this.#fill(id, kind, suggest);
	}

	setHost(id: number, host: string): void {
		const row = this.#row(id);
		if (!row || row.address.kind === 'loopback-port') return;
		this.#pending.delete(id);
		row.address = { ...row.address, host };
	}

	setPort(id: number, port: number): void {
		const row = this.#row(id);
		if (!row) return;
		this.#pending.delete(id);
		row.address = { kind: 'loopback-port', port };
	}

	remove(id: number): void {
		this.#pending.delete(id);
		const index = this.#rows.findIndex((row) => row.id === id);
		if (index !== -1) this.#rows.splice(index, 1);
	}

	#row(id: number): AddressRow | undefined {
		return this.#rows.find((row) => row.id === id);
	}

	/** Fills row `id` with the suggestion for `kind`, unless the row was edited or removed meanwhile. */
	async #fill(id: number, kind: AddressKind, suggest: SuggestAddresses): Promise<void> {
		const request = Symbol(kind);
		this.#pending.set(id, request);
		const suggestions = await suggest();
		if (this.#pending.get(id) !== request) return;
		this.#pending.delete(id);
		const row = this.#row(id);
		if (row) row.address = this.#proposal(id, kind, suggestions);
	}

	#proposal(id: number, kind: AddressKind, s: AddressSuggestions | null): SiteAddress {
		if (!s) return emptyAddress(kind);
		if (kind === 'hostname') return s.hostname;
		if (kind === 'localhost') return s.localhost;
		const port = s.loopbackPort.kind === 'loopback-port' ? s.loopbackPort.port : NaN;
		// The allocator only knows saved sites: a port already in another row (typed, or proposed
		// to an earlier row) stays for the user to choose.
		return this.#portTaken(id, port) ? emptyAddress(kind) : { kind, port };
	}

	#portTaken(id: number, port: number): boolean {
		return this.#rows.some((row) => row.id !== id && row.address.kind === 'loopback-port' && row.address.port === port);
	}
}
