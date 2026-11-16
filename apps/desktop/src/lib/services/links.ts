/** Highest TCP port: a `conflict` parameter above it cannot name a real conflict. */
const MAX_PORT = 65_535;

/** The logs page on the log explaining a service's failure; without a service the page picks one. */
export const serviceLogsHref = (service: string | null) => (service ? `/logs?service=${encodeURIComponent(service)}` : '/logs');

/** The services page with the conflict on `port` highlighted. */
export const portConflictHref = (port: number) => `/services?conflict=${port}`;

/** The port of the `conflict` parameter of the services page, `null` when absent or malformed. */
export function conflictPortParam(value: string | null): number | null {
	if (value === null || !/^\d{1,5}$/.test(value)) return null;
	const port = Number(value);
	return port >= 1 && port <= MAX_PORT ? port : null;
}
