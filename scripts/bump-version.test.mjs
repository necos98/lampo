// `node --test "scripts/*.test.mjs"` (or `pnpm test` from apps/desktop): version sync and updater wiring.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
	CARGO_LOCK,
	CARGO_TOML,
	PACKAGE_JSON,
	TAURI_CONF,
	assertValidVersion,
	bumpCargoLock,
	bumpCargoToml,
	bumpPackageJson,
	lockEntry,
	readWorkspaceVersion,
	workspaceCrates
} from './bump-version.mjs';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const read = (p) => readFileSync(path.join(root, p), 'utf8');
const readJson = (p) => JSON.parse(read(p));

describe('versioni sincronizzate', () => {
	const version = readWorkspaceVersion(read(CARGO_TOML));

	it('package.json ha la versione del workspace Cargo', () => {
		assert.equal(readJson(PACKAGE_JSON).version, version, `${PACKAGE_JSON} != ${CARGO_TOML} (${version}): usa scripts/bump-version.mjs`);
	});

	it('Cargo.lock ha la versione del workspace per ogni crate lampo-*', () => {
		const lock = read(CARGO_LOCK);
		const crates = workspaceCrates(root);
		assert.ok(crates.includes('lampo-desktop') && crates.includes('lampo-cli'), `crate del workspace: ${crates.join(', ')}`);
		for (const name of crates) {
			const match = lock.match(lockEntry(name));
			assert.ok(match, `${name} non trovato in ${CARGO_LOCK}`);
			assert.equal(lock.match(new RegExp(`name = "${name}"\\r?\\nversion = "([^"]+)"`))[1], version, `${CARGO_LOCK} (${name}) != ${version}`);
		}
	});

	it('ogni crate eredita la versione dal workspace', () => {
		for (const member of read(CARGO_TOML).match(/^members\s*=\s*\[([^\]]*)\]/m)[1].matchAll(/"([^"]+)"/g)) {
			const toml = read(path.join(member[1], 'Cargo.toml'));
			assert.match(toml, /^version\.workspace\s*=\s*true/m, `${member[1]}/Cargo.toml deve usare version.workspace = true`);
		}
	});

	it('tauri.conf.json non ha version: la eredita dal crate', () => {
		assert.ok(!('version' in readJson(TAURI_CONF)), `${TAURI_CONF} non deve duplicare la versione`);
	});
});

describe('bump-version', () => {
	it('rifiuta versioni non valide', () => {
		for (const bad of ['', '1.2', 'v1.2.3', '1.2.3.4', undefined]) assert.throws(() => assertValidVersion(bad));
		assertValidVersion('1.2.3');
		assertValidVersion('1.2.3-beta.1');
	});

	it('Cargo.toml: solo [workspace.package] version', () => {
		const raw = '[workspace]\r\nmembers = ["a"]\r\n\r\n[workspace.package]\r\nversion = "0.1.0"\r\nedition = "2024"\r\n\r\n[workspace.dependencies]\r\nserde = { version = "1" }\r\n';
		const out = bumpCargoToml(raw, '0.2.0');
		assert.equal(out, raw.replace('version = "0.1.0"', 'version = "0.2.0"'));
		assert.throws(() => bumpCargoToml('[package]\nversion = "1.0.0"\n', '0.2.0'));
	});

	it('Cargo.lock: tocca solo i crate indicati', () => {
		const raw = '[[package]]\nname = "lampo-cli"\nversion = "0.1.0"\n\n[[package]]\nname = "lampo-cli-extra"\nversion = "0.1.0"\n\n[[package]]\nname = "serde"\nversion = "0.1.0"\n';
		const out = bumpCargoLock(raw, '0.3.0', ['lampo-cli']);
		assert.match(out, /name = "lampo-cli"\nversion = "0\.3\.0"/);
		assert.match(out, /name = "lampo-cli-extra"\nversion = "0\.1\.0"/);
		assert.match(out, /name = "serde"\nversion = "0\.1\.0"/);
		assert.throws(() => bumpCargoLock(raw, '0.3.0', ['lampo-missing']));
	});

	it('package.json: mantiene indentazione e fine riga', () => {
		const raw = '{\r\n\t"name": "desktop",\r\n\t"version": "0.1.0"\r\n}\r\n';
		assert.equal(bumpPackageJson(raw, '1.0.0'), raw.replace('0.1.0', '1.0.0'));
	});
});

describe('updater: configurazione', () => {
	it('tauri.conf.json: NSIS, endpoint latest.json di necos98/lampo, installMode passive', () => {
		const conf = readJson(TAURI_CONF);
		assert.deepEqual(conf.bundle.targets, ['nsis']);
		const up = conf.plugins.updater;
		assert.ok(typeof up.pubkey === 'string' && up.pubkey.length > 0, 'pubkey mancante');
		assert.deepEqual(up.endpoints, ['https://github.com/necos98/lampo/releases/latest/download/latest.json']);
		assert.equal(up.windows.installMode, 'passive');
	});

	it('tauri.bundle.conf.json: artefatti firmati, sidecar e hook NSIS', () => {
		const bundle = readJson('apps/desktop/src-tauri/tauri.bundle.conf.json').bundle;
		assert.equal(bundle.createUpdaterArtifacts, true);
		assert.deepEqual(bundle.externalBin, ['binaries/lampo', 'binaries/lampo-shim', 'binaries/lampo-hosts-helper']);
		const hooks = bundle.windows.nsis.installerHooks;
		assert.ok(existsSync(path.join(root, 'apps/desktop/src-tauri', hooks)), `hook NSIS mancante: ${hooks}`);
		const nsh = read(path.join('apps/desktop/src-tauri', hooks));
		assert.match(nsh, /NSIS_HOOK_PREINSTALL/);
		assert.match(nsh, /lampo\.exe/);
		assert.match(nsh, /lampo-hosts-helper\.exe/);
	});

	it('hook NSIS: installa in %LOCALAPPDATA%\\Programs\\Lampo, mai nella cartella dati', () => {
		const conf = readJson(TAURI_CONF);
		const bundle = readJson('apps/desktop/src-tauri/tauri.bundle.conf.json').bundle;
		// Il default per-utente di Tauri è $LOCALAPPDATA\<productName>: coincide con la cartella
		// dati (APP_DIR_NAME in paths.rs) finché il productName è "Lampo".
		const paths = read('crates/lampo-infra-store/src/paths.rs');
		const dataDir = paths.match(/pub const APP_DIR_NAME: &str = "([^"]+)";/)[1];
		assert.equal(conf.productName, dataDir);
		assert.equal(bundle.windows.nsis.installMode ?? 'currentUser', 'currentUser');
		const nsh = read(path.join('apps/desktop/src-tauri', bundle.windows.nsis.installerHooks));
		assert.ok(nsh.includes(`!define LAMPO_DATA_ROOT_NAME "${dataDir}"`), 'nome della cartella dati nel hook');
		assert.match(nsh, /!define LAMPO_INSTALL_PARENT "Programs"/);
		assert.match(nsh, /!define MUI_CUSTOMFUNCTION_GUIINIT LampoGuiInit/);
		assert.match(nsh, /Function \.onVerifyInstDir/);
		const preinstall = nsh.slice(nsh.indexOf('!macro NSIS_HOOK_PREINSTALL'));
		assert.match(preinstall, /Call LampoAvoidDataRoot\s+SetOutPath \$INSTDIR/);
	});

	it('capability default abilita updater e riavvio', () => {
		const cap = readJson('apps/desktop/src-tauri/capabilities/default.json');
		assert.ok(cap.permissions.includes('updater:default'));
		assert.ok(cap.permissions.includes('process:allow-restart'));
	});

	it('lib.rs registra updater e process dopo single-instance', () => {
		const lib = read('apps/desktop/src-tauri/src/lib.rs');
		const single = lib.indexOf('tauri_plugin_single_instance::init');
		assert.ok(single >= 0 && single < lib.indexOf('tauri_plugin_updater::Builder::new().build()'));
		assert.ok(lib.includes('tauri_plugin_process::init()'));
		assert.match(read('apps/desktop/src-tauri/src/commands/mod.rs'), /commands::app::prepare_update/);
	});
});
