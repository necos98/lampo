// The same rules classify lines for `lampo logs --level` (crates/lampo-domain/src/log_level.rs):
// change both together, their tests share the same cases.

/** Severity a log line is highlighted with; `null` for ordinary lines. */
export type LogLevel = 'error' | 'warn' | null;

/** Error keywords, or an HTTP 5xx status between spaces (access logs). */
const ERROR_PATTERN = /\b(error|fatal|critical|emerg|alert)\b|\s5\d\d\s/i;
const WARN_PATTERN = /\b(warn(ing)?|deprecated|notice)\b/i;

/** Classifies a line once, when it enters the viewer, so rendering never runs a regex. */
export function levelOf(text: string): LogLevel {
	if (ERROR_PATTERN.test(text)) return 'error';
	if (WARN_PATTERN.test(text)) return 'warn';
	return null;
}

/** Which lines the viewer keeps by severity: every line, warnings and errors, or errors only. */
export type LevelFilter = 'all' | 'warn' | 'error';

export const LEVEL_FILTERS: readonly LevelFilter[] = ['all', 'warn', 'error'];

const SEVERITY: Record<Exclude<LogLevel, null>, number> = { warn: 1, error: 2 };
const MIN_SEVERITY: Record<LevelFilter, number> = { all: 0, warn: 1, error: 2 };

/** Whether a line of `level` is kept by `filter`. */
export function passesLevel(level: LogLevel, filter: LevelFilter): boolean {
	return (level ? SEVERITY[level] : 0) >= MIN_SEVERITY[filter];
}

export function isLevelFilter(value: string): value is LevelFilter {
	return (LEVEL_FILTERS as readonly string[]).includes(value);
}
