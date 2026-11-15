import { toAppError, type AppError, type LogLine } from '../api/index.js';
import type { Backend } from '../backend/ports.js';

const INITIAL_LINES = 400;
const MAX_LINES = 3000;
const POLL_MS = 1000;

export interface LogEntry extends LogLine {
	id: number;
}

/**
 * Live tail of one log source: an initial `tailLog`, then `readLogFrom` polling with the offset
 * the backend handed back. One session per viewer; `close()` stops the timer. Pausing stops the
 * polling but keeps the offset, so resuming continues without losing lines.
 */
export class LogSession {
	source = $state<string | null>(null);
	file = $state('');
	lines = $state<LogEntry[]>([]);
	status = $state<'idle' | 'loading' | 'ready' | 'error'>('idle');
	error = $state<AppError | null>(null);
	paused = $state(false);
	filter = $state('');

	readonly #backend: Backend;
	#offset = 0;
	#seq = 0;
	#generation = 0;
	#timer: ReturnType<typeof setTimeout> | undefined;

	constructor(backend: Backend) {
		this.#backend = backend;
	}

	get visible(): LogEntry[] {
		const needle = this.filter.trim().toLowerCase();
		return needle ? this.lines.filter((l) => l.text.toLowerCase().includes(needle)) : this.lines;
	}

	async open(source: string | null): Promise<void> {
		this.close();
		const generation = ++this.#generation;
		this.source = source;
		this.lines = [];
		this.error = null;
		if (!source) {
			this.status = 'idle';
			return;
		}
		this.status = 'loading';
		try {
			const tail = await this.#backend.commands.tailLog({ source, lines: INITIAL_LINES, sinceUnixMs: null });
			if (generation !== this.#generation) return;
			this.file = tail.file;
			this.#offset = tail.nextOffset;
			this.lines = tail.lines.map((l) => this.#entry(l));
			this.status = 'ready';
			this.#schedule(generation);
		} catch (e) {
			if (generation !== this.#generation) return;
			this.error = toAppError(e);
			this.status = 'error';
		}
	}

	pause(): void {
		this.paused = true;
	}

	resume(): void {
		this.paused = false;
	}

	clear(): void {
		this.lines = [];
	}

	close(): void {
		this.#generation++;
		clearTimeout(this.#timer);
	}

	#entry(line: LogLine): LogEntry {
		return { ...line, id: ++this.#seq };
	}

	#schedule(generation: number): void {
		this.#timer = setTimeout(() => void this.#poll(generation), POLL_MS);
	}

	async #poll(generation: number): Promise<void> {
		if (generation !== this.#generation || !this.source) return;
		if (!this.paused) {
			try {
				const next = await this.#backend.commands.readLogFrom(this.source, this.#offset);
				if (generation !== this.#generation) return;
				this.#offset = next.nextOffset;
				if (next.lines.length) this.lines = [...this.lines, ...next.lines.map((l) => this.#entry(l))].slice(-MAX_LINES);
			} catch {
				// A missed poll is not worth interrupting the viewer; the next tick retries.
			}
		}
		this.#schedule(generation);
	}
}
