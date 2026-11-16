/** Extension of exported log files, without the dot. */
export const EXPORT_EXTENSION = 'log';

/** Runs of characters a Windows file name cannot hold (or that read badly in one). */
const UNSAFE_FILE_NAME_CHARS = /[^\w.-]+/g;

/** `error:shop` -> `error-shop.log`: the suggested name of an export of `source`. */
export function exportFileName(source: string): string {
	return `${source.replace(UNSAFE_FILE_NAME_CHARS, '-')}.${EXPORT_EXTENSION}`;
}
