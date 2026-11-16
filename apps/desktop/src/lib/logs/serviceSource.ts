/** Suffix of a service's own error log (`apache-error`, `mysql-8.4-error`). */
const ERROR_LOG_SUFFIX = '-error';

/**
 * The log explaining a service's failure: its error log (where a failed start says why), else
 * its captured output. Only a log with a file counts; `null` when the service has none.
 */
export function serviceLogSource(sources: readonly { id: string; exists: boolean }[], service: string | null): string | null {
	if (service === null) return null;
	const withFile = (id: string) => sources.some((s) => s.id === id && s.exists);
	return [`${service}${ERROR_LOG_SUFFIX}`, service].find(withFile) ?? null;
}
