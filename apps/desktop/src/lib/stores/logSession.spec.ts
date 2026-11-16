import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { LogLine, LogLines } from '../api/index.js';
import { deferred, testBackend } from '../testing/harness.js';
import { LogSession } from './logSession.svelte.js';

const POLL_MS = 1000;
const SOURCE = 'apache';

const line = (text: string): LogLine => ({ text, atUnixMs: 0 });
const batch = (texts: string[], nextOffset: number): LogLines => ({ source: SOURCE, file: 'apache.log', lines: texts.map(line), nextOffset });
const texts = (session: LogSession) => session.lines.map((l) => l.text);

/** A session over a scripted log: `tail` is the first read, `reads` the answers of the next polls. */
function scripted(tail: LogLines, reads: LogLines[] = []) {
	const offsets: number[] = [];
	const { backend } = testBackend({
		tailLog: async () => tail,
		readLogFrom: async (_source, offset) => {
			offsets.push(offset);
			return reads.shift() ?? batch([], offset);
		}
	});
	return { session: new LogSession(backend), offsets };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('LogSession loading', () => {
	it('is idle without a source', async () => {
		const { session } = scripted(batch([], 0));

		await session.open(null);

		expect(session.status).toBe('idle');
		expect(session.lines).toEqual([]);
	});

	it('shows the initial tail and classifies each line', async () => {
		const { session } = scripted(batch(['ok', '[php:error] Fatal'], 2));

		await session.open(SOURCE);

		expect(session.status).toBe('ready');
		expect(session.file).toBe('apache.log');
		expect(texts(session)).toEqual(['ok', '[php:error] Fatal']);
		expect(session.lines.map((l) => l.id)).toEqual([1, 2]);
		session.close();
	});

	it('reports the error when the tail cannot be read', async () => {
		const { backend } = testBackend({
			tailLog: async () => {
				throw { code: 'not-found', message: 'no such log' };
			}
		});
		const session = new LogSession(backend);

		await session.open(SOURCE);

		expect(session.status).toBe('error');
		expect(session.error?.message).toBe('no such log');
	});

	it('ignores the tail of a source that was replaced while loading', async () => {
		const slow = deferred<LogLines>();
		const responses = [slow.promise, Promise.resolve(batch(['new'], 1))];
		const { backend } = testBackend({ tailLog: () => responses.shift()! });
		const session = new LogSession(backend);

		const first = session.open('old');
		await session.open(SOURCE);
		slow.resolve(batch(['old'], 1));
		await first;

		expect(session.source).toBe(SOURCE);
		expect(texts(session)).toEqual(['new']);
		session.close();
	});
});

describe('LogSession polling', () => {
	it('reads on from the offset the backend handed back', async () => {
		const { session, offsets } = scripted(batch(['a'], 5), [batch(['b'], 8), batch(['c'], 9)]);
		await session.open(SOURCE);

		await vi.advanceTimersByTimeAsync(POLL_MS);
		await vi.advanceTimersByTimeAsync(POLL_MS);

		expect(offsets).toEqual([5, 8]);
		expect(texts(session)).toEqual(['a', 'b', 'c']);
		session.close();
	});

	it('survives a failed poll and tries again on the next tick', async () => {
		let calls = 0;
		const { backend } = testBackend({
			tailLog: async () => batch(['a'], 1),
			readLogFrom: async () => {
				if (calls++ === 0) throw { code: 'internal', message: 'busy' };
				return batch(['b'], 2);
			}
		});
		const session = new LogSession(backend);
		await session.open(SOURCE);

		await vi.advanceTimersByTimeAsync(POLL_MS);
		expect(texts(session)).toEqual(['a']);
		await vi.advanceTimersByTimeAsync(POLL_MS);

		expect(texts(session)).toEqual(['a', 'b']);
		expect(session.status).toBe('ready');
		session.close();
	});

	it('stops polling after close()', async () => {
		const { session, offsets } = scripted(batch(['a'], 1));
		await session.open(SOURCE);

		session.close();
		await vi.advanceTimersByTimeAsync(POLL_MS * 3);

		expect(offsets).toEqual([]);
	});

	it('does not poll while hidden and catches up at once when shown', async () => {
		const { session, offsets } = scripted(batch(['a'], 1), [batch(['b'], 2)]);
		await session.open(SOURCE);

		session.setHidden(true);
		await vi.advanceTimersByTimeAsync(POLL_MS * 3);
		expect(offsets).toEqual([]);

		session.setHidden(false);
		await vi.advanceTimersByTimeAsync(0);

		expect(offsets).toEqual([1]);
		expect(texts(session)).toEqual(['a', 'b']);
		session.close();
	});
});

describe('LogSession follow and pause', () => {
	it('follows by default: new lines show at once', async () => {
		const { session } = scripted(batch(['a'], 1), [batch(['b'], 2)]);
		await session.open(SOURCE);

		await vi.advanceTimersByTimeAsync(POLL_MS);

		expect(session.following).toBe(true);
		expect(session.pendingCount).toBe(0);
		expect(texts(session)).toEqual(['a', 'b']);
		session.close();
	});

	it('holds new lines back while paused and counts them', async () => {
		const { session } = scripted(batch(['a'], 1), [batch(['b', 'c'], 3), batch(['d'], 4)]);
		await session.open(SOURCE);

		session.unfollow();
		await vi.advanceTimersByTimeAsync(POLL_MS);
		await vi.advanceTimersByTimeAsync(POLL_MS);

		expect(session.following).toBe(false);
		expect(texts(session)).toEqual(['a']);
		expect(session.pendingCount).toBe(3);
		session.close();
	});

	it('follow() reveals the held lines in order and resumes following', async () => {
		const { session } = scripted(batch(['a'], 1), [batch(['b', 'c'], 3)]);
		await session.open(SOURCE);
		session.unfollow();
		await vi.advanceTimersByTimeAsync(POLL_MS);

		session.follow();

		expect(texts(session)).toEqual(['a', 'b', 'c']);
		expect(session.pendingCount).toBe(0);
		expect(session.following).toBe(true);
		session.close();
	});

	it('opening another source follows again and drops what was held', async () => {
		const { session } = scripted(batch(['a'], 1), [batch(['held'], 2)]);
		await session.open(SOURCE);
		session.unfollow();
		await vi.advanceTimersByTimeAsync(POLL_MS);

		await session.open(SOURCE);

		expect(session.following).toBe(true);
		expect(session.pendingCount).toBe(0);
		expect(texts(session)).toEqual(['a']);
		session.close();
	});

	it('clear() empties the view', async () => {
		const { session } = scripted(batch(['a', 'b'], 2));
		await session.open(SOURCE);

		session.clear();

		expect(session.lines).toEqual([]);
		session.close();
	});
});

describe('LogSession filters', () => {
	it('filters by text, ignoring case and surrounding spaces', async () => {
		const { session } = scripted(batch(['GET /home', 'POST /cart', 'get /about'], 3));
		await session.open(SOURCE);

		session.filter = '  get ';

		expect(session.visible.map((l) => l.text)).toEqual(['GET /home', 'get /about']);
		expect(session.filtering).toBe(true);
		session.close();
	});

	it('shows every line without a filter', async () => {
		const { session } = scripted(batch(['a', 'b'], 2));
		await session.open(SOURCE);

		expect(session.visible).toBe(session.lines);
		expect(session.filtering).toBe(false);
		session.close();
	});

	it('combines the level filter with the text filter', async () => {
		const { session } = scripted(batch(['[php:error] boom', '[php:error] bang', '[php:notice] fine'], 3));
		await session.open(SOURCE);

		session.levelFilter = 'error';
		const errors = session.visible.map((l) => l.text);
		session.filter = 'bang';

		expect(errors).toEqual(['[php:error] boom', '[php:error] bang']);
		expect(session.visible.map((l) => l.text)).toEqual(['[php:error] bang']);
		session.close();
	});
});
