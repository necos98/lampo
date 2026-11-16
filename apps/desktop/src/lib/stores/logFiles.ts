import type { Backend } from '../backend/ports.js';
import { EXPORT_EXTENSION, exportFileName } from '../logs/exportFile.js';
import { messages as m } from '../messages/index.js';
import type { Notifier } from '../services/notifier.js';
import type { CommandRunner } from './runner.js';

/** Writes text to the clipboard; `false` when the clipboard refused it. */
export type CopyText = (text: string) => Promise<boolean>;

/** What the log viewer does with a log outside itself: copy, export, open its file or folder. */
export class LogFileActions {
	readonly #backend: Backend;
	readonly #runner: CommandRunner;
	readonly #notifier: Notifier;
	readonly #copyText: CopyText;

	constructor(backend: Backend, runner: CommandRunner, notifier: Notifier, copyText: CopyText) {
		this.#backend = backend;
		this.#runner = runner;
		this.#notifier = notifier;
		this.#copyText = copyText;
	}

	async copy(lines: readonly string[]): Promise<void> {
		if (await this.#copyText(lines.join('\n'))) this.#notifier.success(m.logs.copied(lines.length));
		else this.#notifier.error(m.common.copyFailed);
	}

	/** Asks where to save `lines`, then writes them; a cancelled picker does nothing. */
	async export(source: string, lines: readonly string[]): Promise<void> {
		let file: string | null;
		try {
			file = await this.#backend.platform.pickSaveFile({
				title: m.logs.exportTitle,
				defaultPath: exportFileName(source),
				filters: [{ name: m.logs.exportFilter, extensions: [EXPORT_EXTENSION] }]
			});
		} catch (e) {
			this.#notifier.error(m.common.actionFailed, e instanceof Error ? e.message : String(e));
			return;
		}
		if (!file) return;
		await this.#runner.run(() => this.#backend.commands.exportLogLines({ file, lines: [...lines] }), { success: m.logs.exported });
	}

	openFile(source: string) {
		return this.#runner.run(() => this.#backend.commands.openLogFile(source), { errors: { validation: m.logs.fileMissing } });
	}

	openFolder(source: string) {
		return this.#runner.run(() => this.#backend.commands.openLogFolder(source), { errors: { validation: m.logs.folderMissing } });
	}
}
