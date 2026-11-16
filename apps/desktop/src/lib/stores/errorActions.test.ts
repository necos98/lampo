import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { AppError } from '../api/index.js';
import type { NoticeActions } from '../services/notifier.js';
import { conflictPort, errorNoticeActions, type ErrorActionLinks, type ErrorActionTexts, type ErrorRecovery } from './errorActions.ts';

const TEXTS: ErrorActionTexts = { retry: 'retry', viewLogs: 'logs', viewConflict: 'conflict' };
const LINKS: ErrorActionLinks = { serviceLogs: (s) => `logs:${s ?? '*'}`, portConflict: (p) => `port:${p}` };

const failure: AppError = { code: 'process-failed', message: 'exited with 1' };
const portBusy: AppError = { code: 'port-in-use', message: 'busy', details: { port: 3307, owner: 'mysqld.exe' } };

/** The actions for `error`, and where clicking them navigated. */
function actionsFor(error: AppError, recovery: ErrorRecovery) {
	const visited: string[] = [];
	const actions = errorNoticeActions(error, recovery, TEXTS, LINKS, (href) => void visited.push(href));
	return { actions, visited };
}

const labels = ({ action, secondaryAction }: NoticeActions) => [action?.label ?? null, secondaryAction?.label ?? null];

test('nothing to offer: a plain error toast', () => {
	assert.deepEqual(labels(actionsFor(failure, {}).actions), [null, null]);
});

test('«Riprova» runs the operation again', () => {
	let runs = 0;
	const { actions } = actionsFor(failure, { retry: () => runs++ });
	assert.deepEqual(labels(actions), ['retry', null]);
	actions.action?.run();
	assert.equal(runs, 1);
});

test('«Vedi log» opens the log of the service named on click', () => {
	let failed: string | null = null;
	const { actions, visited } = actionsFor(failure, { retry: () => {}, serviceLogs: () => failed });
	assert.deepEqual(labels(actions), ['retry', 'logs']);
	failed = 'mysql-5.7';
	actions.secondaryAction?.run();
	assert.deepEqual(visited, ['logs:mysql-5.7']);
});

test('without «Riprova» the link is the main button; an unknown service lets the page pick', () => {
	const { actions, visited } = actionsFor(failure, { serviceLogs: () => null });
	assert.deepEqual(labels(actions), ['logs', null]);
	actions.action?.run();
	assert.deepEqual(visited, ['logs:*']);
});

test('a busy port links to its conflict, in place of the log', () => {
	const { actions, visited } = actionsFor(portBusy, { retry: () => {}, serviceLogs: () => 'mysql-5.7' });
	assert.deepEqual(labels(actions), ['retry', 'conflict']);
	actions.secondaryAction?.run();
	assert.deepEqual(visited, ['port:3307']);
});

test('a busy port links to its conflict for any operation, e.g. serving a project', () => {
	assert.deepEqual(labels(actionsFor(portBusy, {}).actions), ['conflict', null]);
});

test('the conflict port comes only from a well-formed port-in-use error', () => {
	assert.equal(conflictPort(portBusy), 3307);
	assert.equal(conflictPort({ code: 'port-in-use', message: 'busy' }), null);
	assert.equal(conflictPort({ code: 'port-in-use', message: 'busy', details: { port: '3307' } }), null);
	assert.equal(conflictPort({ code: 'port-in-use', message: 'busy', details: null }), null);
	assert.equal(conflictPort({ ...failure, details: { port: 80 } }), null);
	assert.deepEqual(labels(actionsFor({ code: 'port-in-use', message: 'busy' }, { serviceLogs: () => 'apache' }).actions), ['logs', null]);
});
