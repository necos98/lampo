const dateFmt = new Intl.DateTimeFormat('it-IT', { dateStyle: 'medium', timeStyle: 'short' });

export function formatBytes(bytes: number | null): string {
	if (bytes === null) return '';
	if (bytes < 1_000_000) return `${Math.max(1, Math.round(bytes / 1000))} kB`;
	if (bytes < 1_000_000_000) return `${(bytes / 1_000_000).toFixed(bytes < 10_000_000 ? 1 : 0)} MB`;
	return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
}

export function formatDateTime(unixMs: number): string {
	return dateFmt.format(new Date(unixMs));
}

/** Last path segment, for either separator. */
export function baseName(path: string): string {
	return path.split(/[\/]/).filter(Boolean).pop() ?? path;
}
