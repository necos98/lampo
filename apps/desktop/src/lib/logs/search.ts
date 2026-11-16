/** `id` of the log search field, so a shortcut can focus it. */
export const LOG_SEARCH_INPUT_ID = 'log-search';

/** A run of a line's text, marked when it matches the search. */
export interface TextSegment {
	text: string;
	match: boolean;
}

/** Case-insensitive, literal search for one query; built once per query, used on every line. */
export interface LogMatcher {
	test(text: string): boolean;
	/** `text` split into runs, the matching ones marked; one unmarked run when nothing matches. */
	segments(text: string): TextSegment[];
}

const REGEX_SPECIAL = /[.*+?^${}()|[\]\\]/g;

/** The query as a literal pattern: the reader types text, never a regular expression. */
function escapeRegExp(text: string): string {
	return text.replace(REGEX_SPECIAL, '\\$&');
}

/**
 * The matcher of `query`, or `null` for a blank one. A regular expression with the `i` flag
 * (not `toLowerCase`) keeps match offsets right for characters whose lower case is longer.
 */
export function createMatcher(query: string): LogMatcher | null {
	const needle = query.trim();
	if (!needle) return null;
	const source = escapeRegExp(needle);
	const probe = new RegExp(source, 'iu');
	return {
		test: (text) => probe.test(text),
		segments: (text) => {
			const out: TextSegment[] = [];
			let last = 0;
			for (const found of text.matchAll(new RegExp(source, 'giu'))) {
				if (found.index > last) out.push({ text: text.slice(last, found.index), match: false });
				out.push({ text: found[0], match: true });
				last = found.index + found[0].length;
			}
			if (last < text.length || out.length === 0) out.push({ text: text.slice(last), match: false });
			return out;
		}
	};
}

/** Positions in `lines` of the lines `matcher` finds, in order. */
export function matchPositions(lines: readonly { text: string }[], matcher: LogMatcher | null): number[] {
	if (!matcher) return [];
	const out: number[] = [];
	lines.forEach((line, i) => {
		if (matcher.test(line.text)) out.push(i);
	});
	return out;
}

/**
 * The match after (`direction` 1) or before (-1) `current`, wrapping around; from no current
 * match (-1) the first or the last one. `-1` when there is no match at all.
 */
export function stepMatch(current: number, count: number, direction: 1 | -1): number {
	if (count <= 0) return -1;
	if (current < 0 || current >= count) return direction === 1 ? 0 : count - 1;
	return (current + direction + count) % count;
}
