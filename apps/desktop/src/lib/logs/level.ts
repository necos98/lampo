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
