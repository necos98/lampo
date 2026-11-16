import assert from 'node:assert/strict';
import { test } from 'node:test';

import { buildCommands, filterCommands, moveSelection, type PaletteActions, type PaletteGroup, type PaletteLabels, type PaletteSources } from './commands.ts';

const labels: PaletteLabels = {
	title: 'Palette',
	description: '',
	placeholder: '',
	empty: '',
	groups: { general: 'Generale', projects: 'Progetti', services: 'Servizi' },
	newProject: 'Nuovo progetto',
	goTo: { projects: 'Vai a Progetti', services: 'Vai a Servizi', packages: 'Vai a Pacchetti', logs: 'Vai a Log', settings: 'Apri le impostazioni' },
	showHelp: 'Scorciatoie',
	openProject: (name) => `Apri ${name}`,
	activateProject: (name) => `Attiva ${name}`,
	deactivateProject: (name) => `Disattiva ${name}`,
	startAll: 'Avvia tutti',
	stopAll: 'Ferma tutti',
	startService: (label) => `Avvia ${label}`,
	stopService: (label) => `Ferma ${label}`
};

const sources: PaletteSources = {
	sections: [
		{ id: 'projects', href: '/projects' },
		{ id: 'settings', href: '/settings' }
	],
	projects: [
		{ id: 'shop', name: 'Negozio', active: true, folderExists: true },
		{ id: 'blog', name: 'Blog', active: false, folderExists: true },
		{ id: 'gone', name: 'Vecchio', active: false, folderExists: false }
	],
	services: [
		{ service: 'apache', label: 'Apache', state: 'running' },
		{ service: 'mysql-8.4', label: '', state: 'stopped' },
		{ service: 'redis', label: 'Redis', state: 'starting' }
	]
};

/** Actions that record what each command asked for. */
function recorder() {
	const calls: unknown[][] = [];
	const actions: PaletteActions = {
		navigate: (href) => calls.push(['navigate', href]),
		newProject: () => calls.push(['newProject']),
		setProjectActive: (id, active) => calls.push(['setProjectActive', id, active]),
		actOnServices: (action, services) => calls.push(['actOnServices', action, services]),
		showHelp: () => calls.push(['showHelp'])
	};
	return { calls, actions };
}

/** Runs the command labelled `label` built from `from`, returning the actions it asked for. */
function runLabel(label: string, from: PaletteSources = sources) {
	const { calls, actions } = recorder();
	const command = buildCommands(from, actions, labels).find((c) => c.label === label);
	assert.ok(command, `no command «${label}»`);
	command.run();
	return calls;
}

const labelsOf = (from: PaletteSources = sources) => buildCommands(from, recorder().actions, labels).map((c) => c.label);

test('commands are grouped: general, then projects, then services', () => {
	const order: PaletteGroup[] = ['general', 'projects', 'services'];
	const groups = buildCommands(sources, recorder().actions, labels).map((c) => order.indexOf(c.group));
	assert.deepEqual(groups, [...groups].sort((a, b) => a - b));
	assert.deepEqual([...new Set(groups)], [0, 1, 2]);
});

test('ids are unique', () => {
	const ids = buildCommands(sources, recorder().actions, labels).map((c) => c.id);
	assert.equal(new Set(ids).size, ids.length);
});

test('general commands open the create screen, each section and the help', () => {
	assert.deepEqual(runLabel('Nuovo progetto'), [['newProject']]);
	assert.deepEqual(runLabel('Vai a Progetti'), [['navigate', '/projects']]);
	assert.deepEqual(runLabel('Apri le impostazioni'), [['navigate', '/settings']]);
	assert.deepEqual(runLabel('Scorciatoie'), [['showHelp']]);
});

test('a project opens, and toggles to the opposite of its state', () => {
	assert.deepEqual(runLabel('Apri Negozio'), [['navigate', '/projects/shop']]);
	assert.deepEqual(runLabel('Disattiva Negozio'), [['setProjectActive', 'shop', false]]);
	assert.deepEqual(runLabel('Attiva Blog'), [['setProjectActive', 'blog', true]]);
});

test('a project with a missing folder can be opened but not activated', () => {
	assert.ok(labelsOf().includes('Apri Vecchio'));
	assert.ok(!labelsOf().includes('Attiva Vecchio'));
});

test('a project id is escaped in its address', () => {
	const odd = { ...sources, projects: [{ id: 'a/b c', name: 'Strano', active: false, folderExists: true }] };
	assert.deepEqual(runLabel('Apri Strano', odd), [['navigate', '/projects/a%2Fb%20c']]);
});

test('a service that is up is stopped, any other is started; all of them at once too', () => {
	assert.deepEqual(runLabel('Ferma Apache'), [['actOnServices', 'stop', ['apache']]]);
	assert.deepEqual(runLabel('Ferma Redis'), [['actOnServices', 'stop', ['redis']]], 'a starting service is stopped');
	assert.deepEqual(runLabel('Avvia mysql-8.4'), [['actOnServices', 'start', ['mysql-8.4']]], 'no label: the service name');
	assert.deepEqual(runLabel('Avvia tutti'), [['actOnServices', 'start', []]]);
	assert.deepEqual(runLabel('Ferma tutti'), [['actOnServices', 'stop', []]]);
});

test('no planned service, no service command', () => {
	const commands = buildCommands({ ...sources, services: [] }, recorder().actions, labels);
	assert.ok(commands.every((c) => c.group !== 'services'));
});

test('the filter needs every word, in any order and case, in the label or the keywords', () => {
	const commands = buildCommands(sources, recorder().actions, labels);
	const matching = (query: string) => filterCommands(commands, query).map((c) => c.label);
	assert.deepEqual(matching('negozio apri'), ['Apri Negozio']);
	assert.deepEqual(matching('  FERMA   apache '), ['Ferma Apache']);
	assert.deepEqual(matching('shop'), ['Apri Negozio', 'Disattiva Negozio'], 'the project id is a keyword');
	assert.deepEqual(matching('nulla di simile'), []);
	assert.equal(filterCommands(commands, '   ').length, commands.length, 'a blank query keeps them all');
});

test('the selection wraps around at both ends', () => {
	assert.equal(moveSelection(0, 1, 3), 1);
	assert.equal(moveSelection(2, 1, 3), 0);
	assert.equal(moveSelection(0, -1, 3), 2);
	assert.equal(moveSelection(0, 1, 0), 0, 'no rows');
});
