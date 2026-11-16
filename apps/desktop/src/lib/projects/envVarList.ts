/** One row of the environment variables editor; `id` stays the same while rows are edited or removed. */
export interface EnvVarRow {
	readonly id: number;
	name: string;
	value: string;
}

/**
 * What is wrong with a row, checked here only for immediate feedback (the backend decides: it
 * also refuses the names Lampo manages, `${` in a value, …).
 */
export type EnvVarProblem = 'name-required' | 'name-syntax' | 'duplicate' | 'multiline';

/** The part of a row a problem is about. */
export type EnvVarPart = 'name' | 'value';

/** Same syntax as the backend's `EnvVarName`: letters, digits and `_`, not starting with a digit. */
const NAME_SYNTAX = /^[A-Za-z_][A-Za-z0-9_]*$/;
const LINE_BREAK = /[\r\n]/;

export function problemPart(problem: EnvVarProblem): EnvVarPart {
	return problem === 'multiline' ? 'value' : 'name';
}

/** Whether two names are the same variable: Windows compares them case-insensitively. */
export function sameEnvVarName(a: string, b: string): boolean {
	return a.trim().toUpperCase() === b.trim().toUpperCase();
}

/** A row with neither a name nor a value is not a variable: it is left out, never an error. */
function isBlank(row: EnvVarRow): boolean {
	return row.name.trim() === '' && row.value === '';
}

const byName = ([a]: [string, string], [b]: [string, string]) => (a < b ? -1 : a > b ? 1 : 0);

/**
 * The environment variables of a project draft, edited by row id, so that editing a name never
 * moves or re-creates its row. `rows` stores the rows; pass a `$state` array so the UI is reactive.
 */
export class EnvVarList {
	readonly #rows: EnvVarRow[];
	#nextId = 0;

	constructor(rows: EnvVarRow[] = []) {
		this.#rows = rows;
	}

	get rows(): readonly EnvVarRow[] {
		return this.#rows;
	}

	/** The variables by name (trimmed), sorted as the backend sends them; blank rows left out. */
	get values(): Record<string, string> {
		const entries = this.#rows.filter((row) => !isBlank(row)).map((row): [string, string] => [row.name.trim(), row.value]);
		return Object.fromEntries(entries.sort(byName));
	}

	/** The problem of each row that has one, by row id. */
	get problems(): ReadonlyMap<number, EnvVarProblem> {
		const problems = new Map<number, EnvVarProblem>();
		const seen = new Set<string>();
		for (const row of this.#rows) {
			if (isBlank(row)) continue;
			const name = row.name.trim();
			const problem = this.#problemOf(name, row.value, seen);
			if (problem) problems.set(row.id, problem);
			seen.add(name.toUpperCase());
		}
		return problems;
	}

	get valid(): boolean {
		return this.problems.size === 0;
	}

	/** Replaces every row with `vars`, in their order. */
	load(vars: Readonly<Record<string, string>>): void {
		this.#rows.splice(0, this.#rows.length, ...Object.entries(vars).map(([name, value]) => ({ id: this.#nextId++, name, value })));
	}

	/** Appends an empty row and returns its id. */
	add(): number {
		const id = this.#nextId++;
		this.#rows.push({ id, name: '', value: '' });
		return id;
	}

	setName(id: number, name: string): void {
		const row = this.#row(id);
		if (row) row.name = name;
	}

	setValue(id: number, value: string): void {
		const row = this.#row(id);
		if (row) row.value = value;
	}

	remove(id: number): void {
		const index = this.#rows.findIndex((row) => row.id === id);
		if (index !== -1) this.#rows.splice(index, 1);
	}

	#row(id: number): EnvVarRow | undefined {
		return this.#rows.find((row) => row.id === id);
	}

	/** A repeated name is reported on the rows after the first one, which is kept. */
	#problemOf(name: string, value: string, seen: ReadonlySet<string>): EnvVarProblem | null {
		if (name === '') return 'name-required';
		if (!NAME_SYNTAX.test(name)) return 'name-syntax';
		if (seen.has(name.toUpperCase())) return 'duplicate';
		if (LINE_BREAK.test(value)) return 'multiline';
		return null;
	}
}
