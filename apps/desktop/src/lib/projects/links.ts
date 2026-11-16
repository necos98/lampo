/** The logs a site keeps itself: Apache errors, Apache accesses, PHP errors. */
export type SiteLogKind = 'error' | 'access' | 'php';

/** The site's logs, in the order the log viewer lists them. */
export const SITE_LOG_KINDS: readonly SiteLogKind[] = ['error', 'access', 'php'];

/** The logs page opened on one of the project's logs (its Apache error log by default). */
export const projectLogsHref = (projectId: string, kind: SiteLogKind = 'error') => `/logs?source=${kind}:${encodeURIComponent(projectId)}`;
