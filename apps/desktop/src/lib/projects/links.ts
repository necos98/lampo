/** The logs page opened on the project's Apache error log. */
export const projectLogsHref = (projectId: string) => `/logs?source=error:${encodeURIComponent(projectId)}`;
