import assert from 'node:assert/strict';
import { test } from 'node:test';

import { EnvVarList, problemPart, sameEnvVarName } from './envVarList.ts';

function listOf(vars: Record<string, string>): EnvVarList {
	const list = new EnvVarList();
	list.load(vars);
	return list;
}

const problemsByName = (list: EnvVarList) => list.rows.flatMap((row) => (list.problems.has(row.id) ? [[row.name, list.problems.get(row.id)]] : []));

test('loading and reading back gives the same variables', () => {
	const vars = { APP_ENV: 'local', API_KEY: 'a=b c' };
	assert.deepEqual(listOf(vars).values, vars);
});

test('values are sorted by name, as the backend sends them, so a re-added row is not a change', () => {
	const list = listOf({ A: '1', B: '2' });
	list.remove(list.rows[0].id);
	list.setName(list.add(), 'A');
	list.setValue(list.rows[1].id, '1');
	assert.equal(JSON.stringify(list.values), JSON.stringify({ A: '1', B: '2' }));
});

test('names are trimmed, values are kept as typed', () => {
	const list = new EnvVarList();
	const id = list.add();
	list.setName(id, '  APP_DEBUG ');
	list.setValue(id, ' true ');
	assert.deepEqual(list.values, { APP_DEBUG: ' true ' });
	assert.equal(list.valid, true);
});

test('a blank row is left out and is not an error', () => {
	const list = listOf({ APP_ENV: 'local' });
	list.add();
	assert.deepEqual(list.values, { APP_ENV: 'local' });
	assert.equal(list.valid, true);
});

test('edits and removals act on the row id, not on its position', () => {
	const list = listOf({ A: '1', B: '2', C: '3' });
	const [a, b, c] = list.rows.map((row) => row.id);
	list.remove(b);
	list.setValue(c, 'changed');
	list.setName(a, 'Z');
	assert.deepEqual(
		list.rows.map((row) => row.id),
		[a, c]
	);
	assert.deepEqual(list.values, { C: 'changed', Z: '1' });
});

test('an unknown id is ignored', () => {
	const list = listOf({ A: '1' });
	list.setName(99, 'B');
	list.setValue(99, 'x');
	list.remove(99);
	assert.deepEqual(list.values, { A: '1' });
});

test('names must follow the shell syntax', () => {
	const list = new EnvVarList();
	for (const name of ['1ABC', 'MY-VAR', 'A B', 'ÀCCENTO', 'OK_1', '_PRIVATE']) list.setName(list.add(), name);
	assert.deepEqual(problemsByName(list), [
		['1ABC', 'name-syntax'],
		['MY-VAR', 'name-syntax'],
		['A B', 'name-syntax'],
		['ÀCCENTO', 'name-syntax']
	]);
	assert.equal(list.valid, false);
});

test('a value without a name needs one', () => {
	const list = new EnvVarList();
	list.setValue(list.add(), 'orphan');
	assert.deepEqual(problemsByName(list), [['', 'name-required']]);
});

test('a name repeated in any case is reported on the later rows', () => {
	const list = listOf({ APP_ENV: 'local' });
	list.setName(list.add(), 'app_env');
	list.setName(list.add(), 'App_Env ');
	assert.deepEqual(problemsByName(list), [
		['app_env', 'duplicate'],
		['App_Env ', 'duplicate']
	]);
});

test('values must be a single line', () => {
	const list = listOf({ A: 'one\ntwo', B: 'one\rtwo', C: 'tab\tok' });
	assert.deepEqual(problemsByName(list), [
		['A', 'multiline'],
		['B', 'multiline']
	]);
});

test('each problem points at the part of the row to fix', () => {
	assert.equal(problemPart('multiline'), 'value');
	assert.equal(problemPart('name-syntax'), 'name');
	assert.equal(problemPart('duplicate'), 'name');
	assert.equal(problemPart('name-required'), 'name');
});

test('names are compared case-insensitively and trimmed', () => {
	assert.equal(sameEnvVarName('app_env', ' APP_ENV'), true);
	assert.equal(sameEnvVarName('APP_ENV', 'APP_ENVX'), false);
});
