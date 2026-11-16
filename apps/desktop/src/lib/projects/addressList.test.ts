import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { AddressSuggestions, SiteAddress } from '../api/index.js';
import { AddressList, type AddressRow } from './addressList.ts';

const suggestions = (port: number): AddressSuggestions => {
	const hostname: SiteAddress = { kind: 'hostname', host: 'blog.test' };
	const localhost: SiteAddress = { kind: 'localhost', host: 'blog.localhost' };
	const loopbackPort: SiteAddress = { kind: 'loopback-port', port };
	return { hostname, localhost, loopbackPort, noElevation: [localhost, loopbackPort], standard: [hostname, localhost], hostsOnly: [hostname], defaultAddressing: 'no-elevation' };
};

/** A `suggest` whose calls stay pending until resolved one by one, in any order. */
function controlledSuggest() {
	const calls: ((value: AddressSuggestions | null) => void)[] = [];
	const suggest = () => new Promise<AddressSuggestions | null>((resolve) => calls.push(resolve));
	return { suggest, calls };
}

const ports = (list: AddressList) => list.values.map((a) => (a.kind === 'loopback-port' ? a.port : null));

test('two quick adds of a port never get the same one, whatever order the suggestions arrive in', async () => {
	const list = new AddressList();
	const { suggest, calls } = controlledSuggest();

	const first = list.add('loopback-port', suggest);
	const second = list.add('loopback-port', suggest);
	assert.equal(list.rows.length, 2, 'both rows appear before any suggestion arrives');

	calls[1](suggestions(8001));
	calls[0](suggestions(8001));
	await Promise.all([first, second]);

	assert.deepEqual(ports(list), [NaN, 8001]);
});

test('a port already in a row counts as taken', async () => {
	const list = new AddressList();
	list.load([{ kind: 'loopback-port', port: 8001 }]);

	await list.add('loopback-port', async () => suggestions(8001));
	await list.add('loopback-port', async () => suggestions(8002));

	assert.deepEqual(ports(list), [8001, NaN, 8002]);
});

test('a row keeps its id while rows before it are removed', async () => {
	const list = new AddressList();
	list.load([
		{ kind: 'localhost', host: 'a.localhost' },
		{ kind: 'localhost', host: 'b.localhost' }
	]);
	const [a, b] = list.rows.map((r) => r.id);
	const { suggest, calls } = controlledSuggest();

	const change = list.changeKind(b, 'loopback-port', suggest);
	list.remove(a);
	calls[0](suggestions(8005));
	await change;

	assert.deepEqual(list.rows.map((r) => r.id), [b]);
	assert.deepEqual(list.values, [{ kind: 'loopback-port', port: 8005 }]);
});

test('a suggestion for a removed row is dropped', async () => {
	const list = new AddressList();
	list.load([{ kind: 'localhost', host: 'a.localhost' }]);
	const { suggest, calls } = controlledSuggest();

	const added = list.add('hostname', suggest);
	list.remove(list.rows[1].id);
	calls[0](suggestions(8001));
	await added;

	assert.deepEqual(list.values, [{ kind: 'localhost', host: 'a.localhost' }]);
});

test('only the latest kind change of a row is applied', async () => {
	const list = new AddressList();
	list.load([{ kind: 'localhost', host: 'a.localhost' }]);
	const id = list.rows[0].id;
	const { suggest, calls } = controlledSuggest();

	const toPort = list.changeKind(id, 'loopback-port', suggest);
	const toHostname = list.changeKind(id, 'hostname', suggest);
	calls[1](suggestions(8001));
	calls[0](suggestions(8001));
	await Promise.all([toPort, toHostname]);

	assert.deepEqual(list.values, [{ kind: 'hostname', host: 'blog.test' }]);
});

test('what the user types before the suggestion arrives is kept', async () => {
	const list = new AddressList();
	const { suggest, calls } = controlledSuggest();

	const added = list.add('localhost', suggest);
	const id = list.rows[0].id;
	list.setHost(id, 'mine.localhost');
	calls[0](suggestions(8001));
	await added;

	assert.deepEqual(list.values, [{ kind: 'localhost', host: 'mine.localhost' }]);
});

test('changing a row to the kind it already has asks for nothing', async () => {
	const list = new AddressList();
	list.load([{ kind: 'localhost', host: 'a.localhost' }]);
	let calls = 0;

	await list.changeKind(list.rows[0].id, 'localhost', async () => {
		calls++;
		return suggestions(8001);
	});

	assert.equal(calls, 0);
	assert.deepEqual(list.values, [{ kind: 'localhost', host: 'a.localhost' }]);
});

test('without suggestions the new row stays empty for the user to fill', async () => {
	const list = new AddressList();

	await list.add('localhost', async () => null);
	await list.add('loopback-port', async () => null);

	assert.deepEqual(list.values, [
		{ kind: 'localhost', host: '' },
		{ kind: 'loopback-port', port: NaN }
	]);
});

test('setHost leaves a port row alone and setPort turns the row into a port', () => {
	const list = new AddressList();
	list.load([
		{ kind: 'loopback-port', port: 8001 },
		{ kind: 'hostname', host: 'blog.test' }
	]);
	const [port, host] = list.rows.map((r) => r.id);

	list.setHost(port, 'ignored');
	list.setPort(host, 8002);

	assert.deepEqual(list.values, [
		{ kind: 'loopback-port', port: 8001 },
		{ kind: 'loopback-port', port: 8002 }
	]);
});

test('load replaces the rows with fresh ids and drops pending suggestions', async () => {
	const list = new AddressList();
	const { suggest, calls } = controlledSuggest();
	const added = list.add('loopback-port', suggest);
	const before = list.rows[0].id;

	list.load([{ kind: 'localhost', host: 'saved.localhost' }]);
	calls[0](suggestions(8001));
	await added;

	assert.notEqual(list.rows[0].id, before);
	assert.deepEqual(list.values, [{ kind: 'localhost', host: 'saved.localhost' }]);
});

test('rows are kept in the array it was given', () => {
	const rows: AddressRow[] = [];
	const list = new AddressList(rows);
	list.load([{ kind: 'localhost', host: 'a.localhost' }]);
	assert.equal(rows.length, 1);
});
