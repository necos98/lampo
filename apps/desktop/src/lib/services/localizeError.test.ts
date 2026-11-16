import assert from 'node:assert/strict';
import { describe, test } from 'node:test';

import { it as m } from '../messages/it.ts';
import { isReportedError, localizeError, type ReportedError } from './localizeError.ts';

const error = (code: string, details?: unknown, message = 'english text'): ReportedError => ({ code, message, details });

describe('localizeError', () => {
	test('a known code without details gets its generic Italian text', () => {
		assert.equal(localizeError(error('timeout'), m), 'Tempo scaduto.');
		assert.equal(localizeError(error('io', null), m), m.errors.io);
	});

	test('a code with no Italian text falls back to the English message', () => {
		assert.equal(localizeError(error('quota-exceeded', undefined, 'disk quota exceeded'), m), 'disk quota exceeded');
		assert.equal(localizeError(error('toString'), m), 'english text', 'inherited properties are not texts');
	});

	test('port-in-use names the port and, when known, its owner', () => {
		assert.equal(localizeError(error('port-in-use', { port: 8080, owner: 'skype.exe' }), m), 'La porta 8080 è già in uso da skype.exe.');
		assert.equal(localizeError(error('port-in-use', { port: 80, owner: null }), m), 'La porta 80 è già in uso da un altro programma.');
		assert.equal(localizeError(error('port-in-use', { port: '80' }), m), m.errors['port-in-use'], 'a malformed port is ignored');
	});

	test('not-found names the site, project or package', () => {
		assert.equal(localizeError(error('not-found', { entity: 'site', id: 'shop.feature' }), m), 'Il sito «shop.feature» non esiste.');
		assert.equal(localizeError(error('not-found', { entity: 'project', id: 'shop' }), m), 'Il progetto «shop» non esiste.');
		assert.equal(localizeError(error('not-found', { entity: 'hosts helper', id: 'x' }), m), m.errors['not-found'], 'an unknown entity keeps the generic text');
		assert.equal(localizeError(error('not-found', { entity: 'constructor', id: 'x' }), m), m.errors['not-found']);
		assert.equal(localizeError(error('not-found', { entity: 'site' }), m), m.errors['not-found']);
	});

	test('package-missing names the package, the add-on, the runtime or the tool', () => {
		assert.equal(localizeError(error('package-missing', { package: 'PHP 8.3' }), m), 'PHP 8.3 non è installato: installalo dalla pagina Pacchetti.');
		assert.equal(localizeError(error('package-missing', { package: 'xdebug', php: '8.3' }), m), 'xdebug per PHP 8.3 non è installato: installalo dalla pagina Pacchetti.');
		assert.equal(
			localizeError(error('package-missing', { package: 'php-5.6.40', redistributable: 'Visual C++ 2012 (x64)', downloadUrl: 'https://example.test' }), m),
			'php-5.6.40 richiede Visual C++ 2012 (x64), che non è installato.'
		);
		assert.equal(localizeError(error('package-missing', { tool: 'git' }), m), m.errorDetails.toolMissing.git);
		assert.equal(localizeError(error('package-missing', { tool: 'vscode' }), m), m.errorDetails.toolMissing.vscode);
		assert.equal(localizeError(error('package-missing', { tool: 'svn' }), m), m.errors['package-missing']);
		assert.equal(localizeError(error('package-missing', { package: '  ' }), m), m.errors['package-missing'], 'a blank name is ignored');
	});

	test('a package still in use lists who uses it', () => {
		assert.equal(localizeError(error('conflict', { package: 'php-8.3.14', usedBy: ['blog', 'shop'] }), m), 'php-8.3.14 è in uso da blog, shop.');
		assert.equal(localizeError(error('conflict', { package: 'php-8.3.14', usedBy: [] }), m), m.errors.conflict);
		assert.equal(localizeError(error('conflict', { usedBy: ['shop'] }), m), m.errors.conflict);
	});

	test('config-rejected names the sites left out', () => {
		assert.equal(
			localizeError(error('config-rejected', { output: 'Syntax error', excludedSites: ['shop', 'blog'] }), m),
			'Apache ha rifiutato la configurazione di shop, blog: questi siti non sono serviti, gli altri sì.'
		);
		assert.equal(localizeError(error('config-rejected', { output: 'Syntax error' }), m), m.errors['config-rejected']);
	});

	test('a missing project folder is named', () => {
		assert.equal(localizeError(error('validation', { folder: 'C:\\www\\shop' }), m), 'La cartella del progetto non esiste: C:\\www\\shop');
		assert.equal(localizeError(error('validation', { field: 'apacheDirectives', reason: 'line 1' }), m), m.errors.validation);
	});

	test('details that are not an object are ignored', () => {
		for (const details of ['port 80', 80, ['shop'], null]) {
			assert.equal(localizeError(error('port-in-use', details), m), m.errors['port-in-use']);
		}
	});
});

describe('isReportedError', () => {
	test('accepts any code and message pair, rejects the rest', () => {
		assert.equal(isReportedError({ code: 'new-code', message: 'x' }), true);
		assert.equal(isReportedError({ code: 'io' }), false);
		assert.equal(isReportedError({ code: 3, message: 'x' }), false);
		assert.equal(isReportedError('boom'), false);
		assert.equal(isReportedError(null), false);
	});
});
