import { toAppError, type AppError, type LogLine } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { levelOf, passesLevel, type LevelFilter, type LogLevel } from '../logs/level.js';
import { Poller } from '../logs/poller.js';
import { TailBuffer } from '../logs/tailBuffer.js';

const INITIAL_LINES = 400;
const MAX_LINES = 3000;
const POLL_MS = 1000;

export interface LogEntry extends LogLine {
	id: number;
	/** Computed once when the line arrives, never while rendering. */
	level: LogLevel;
}

/**
 * Live tail of one log source: an initial `tailLog`, then `readLogFrom` polling with the offset
 * the backend handed back. One session per viewer; `close()` stops the polling, `dispose()`
 * stops it for good when the viewer goes away.
 *
 * Polling runs whether or not the reader follows the tail, so no line is lost; while not
 * following, new lines are held back (see `TailBuffer`) and counted in `pendingCount`. A hidden
 * window does not poll at all: the offset is kept and the next poll catches up.
 */
export class LogSession {
	source = $state<string | null>(null);
	file = $state('');
	/** Replaced as a whole on every change: entries are immutable, so no deep proxy is needed. */
	lines = $state.raw<readonly LogEntry[]>([]);
	pendingCount = $state(0);
	following = $state(true);
	status = $state<'idle' | 'loading' | 'ready' | 'error'>('idle');
	error = $state<AppError | null>(null);
	filter = $state('');
	levelFilter = $state<LevelFilter>('all');

	/** The lines that pass both the text filter and the level filter. */
	readonly visible = $derived.by((): readonly LogEntry[] => {
		const needle = this.filter.trim().toLowerCase();
		const level = this.levelFilter;
		if (!needle && level === 'all') return this.lines;
		return this.lines.filter((l) => passesLevel(l.level, level) && (!needle || l.text.toLowerCase().includes(needle)));
	});

	/** Whether a filter may be hiding lines. */
	get filtering(): boolean {
		return this.filter.trim() !== '' || this.levelFilter !== 'all';
	}

	readonly #backend: Backend;
	readonly #buffer = new TailBuffer<LogEntry>(MAX_LINES);
	readonly #poller = new Poller(() => this.#poll(), POLL_MS);
	#offset = 0;
	#seq = 0;
	#generation = 0;
	#disposed = false;

	constructor(backend: Backend) {
		this.#backend = backend;
	}

	async open(source: string | null): Promise<void> {
		if (this.#disposed) return;
		this.close();
		const generation = ++this.#generation;
		this.source = source;
		this.error = null;
		this.#buffer.reset([]);
		this.#buffer.follow();
		this.#sync();
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
			this.#buffer.reset(this.#entries(tail.lines));
			this.#sync();
			this.status = 'ready';
			this.#poller.start();
		} catch (e) {
			if (generation !== this.#generation) return;
			this.error = toAppError(e);
			this.status = 'error';
		}
	}

	/** Shows the lines that arrived meanwhile and sticks to the newest again. */
	follow(): void {
		this.#buffer.follow();
		this.#sync();
	}

	/** Freezes the visible lines, e.g. because the reader scrolled up. */
	unfollow(): void {
		this.#buffer.unfollow();
		this.#sync();
	}

	clear(): void {
		this.#buffer.clear();
		this.#sync();
	}

	/** Pauses polling while the window is hidden; showing it polls at once. */
	setHidden(hidden: boolean): void {
		this.#poller.setHidden(hidden);
	}

	close(): void {
		this.#generation++;
		this.#poller.stop();
	}

	/** Closes for good: the viewer is gone, so a late `open()` (a list that arrived after) is ignored. */
	dispose(): void {
		this.#disposed = true;
		this.close();
	}

	#entries(lines: LogLine[]): LogEntry[] {
		return lines.map((line) => ({ ...line, id: ++this.#seq, level: levelOf(line.text) }));
	}

	/** Mirrors the buffer into the reactive fields; `lines` only changes when the buffer's does. */
	#sync(): void {
		if (this.lines !== this.#buffer.lines) this.lines = this.#buffer.lines;
		this.pendingCount = this.#buffer.pendingCount;
		this.following = this.#buffer.following;
	}

	async #poll(): Promise<void> {
		const generation = this.#generation;
		const source = this.source;
		if (!source) return;
		try {
			const next = await this.#backend.commands.readLogFrom(source, this.#offset);
			if (generation !== this.#generation) return;
			this.#offset = next.nextOffset;
			this.#buffer.push(this.#entries(next.lines));
			this.#sync();
		} catch {
			// A missed poll is not worth interrupting the viewer; the next tick retries.
		}
	}
}
