import type { TaskOutcome, TaskProgressPayload } from '../api/index.js';
import type { Backend } from '../backend/ports.js';
import { createMockBackend } from '../backend/mock/index.js';
import { MockEmitter } from '../backend/mock/emitter.js';
import type { Notifier } from '../services/notifier.js';
import { CommandRunner } from '../stores/runner.js';

/** The mock backend with an event bus the test drives by hand (the mock's own one is private). */
export interface TestBackend {
	backend: Backend;
	progress(payload: TaskProgressPayload): void;
	finish(taskId: string, outcome: TaskOutcome): void;
}

export function testBackend(overrides: Partial<Backend['commands']> = {}): TestBackend {
	const mock = createMockBackend({ fresh: false });
	const emitter = new MockEmitter();
	return {
		backend: { ...mock, commands: { ...mock.commands, ...overrides }, events: emitter.bus() },
		progress: (payload) => emitter.emit('task-progress', payload),
		finish: (taskId, outcome) => emitter.emit('task-finished', { taskId, outcome })
	};
}

/** A notifier that only remembers what it was told. */
export function recordingNotifier(): Notifier & { errors: string[] } {
	const errors: string[] = [];
	return {
		errors,
		success: () => {},
		info: () => {},
		warning: () => {},
		error: (message) => void errors.push(message)
	};
}

export function testRunner(notifier: Notifier = recordingNotifier()): CommandRunner {
	return new CommandRunner(notifier, () => {}, () => {});
}

/** An operation that stays pending until it is settled by hand. */
export function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (reason: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}
