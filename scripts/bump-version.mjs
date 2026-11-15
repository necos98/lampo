// The only way to change Lampo's version: `node scripts/bump-version.mjs X.Y.Z` (or
// `pnpm bump X.Y.Z` from apps/desktop). Updates:
//   - Cargo.toml (root): `[workspace.package] version`, inherited by every crate;
//   - Cargo.lock: the entry of every workspace crate (lampo-*);
//   - apps/desktop/package.json: `version`.
// apps/desktop/src-tauri/tauri.conf.json has no `version`: Tauri takes the crate's (inherited)
// version. No dependencies; all new contents are computed before any file is written.
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const VERSION_PATTERN = /^\d+\.\d+\.\d+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$/;

export const CARGO_TOML = 'Cargo.toml';
export const CARGO_LOCK = 'Cargo.lock';
export const PACKAGE_JSON = 'apps/desktop/package.json';
export const TAURI_CONF = 'apps/desktop/src-tauri/tauri.conf.json';

export function assertValidVersion(version) {
	if (typeof version !== 'string' || !VERSION_PATTERN.test(version)) {
		throw new Error(`Versione non valida: "${version}" (atteso x.y.z, es. 0.2.0)`);
	}
}

const eolOf = (raw) => (raw.includes('\r\n') ? '\r\n' : '\n');
const escapeRegExp = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

// Replaces the only match of `pattern` (group 1 = text before the version, group 2 = after).
function replaceOnce(raw, pattern, version, label) {
	const count = (raw.match(new RegExp(pattern.source, `${pattern.flags.replace('g', '')}g`)) ?? []).length;
	if (count !== 1) {
		throw new Error(`${label}: attesa 1 occorrenza della versione, trovate ${count}`);
	}
	return raw.replace(pattern, `$1${version}$2`);
}

/**
 * Pattern of `key = "…"` inside the TOML table `[header]`, up to the next table header (a BOM
 * before the first header is allowed). Group 1 / 3 surround the value (group 2).
 */
export const tableKey = (header, key) =>
	new RegExp(`((?:^|\\n)\\uFEFF?\\[${escapeRegExp(header)}\\][ \\t]*\\r?\\n(?:(?!\\[)[^\\n]*\\n)*?${escapeRegExp(key)}[ \\t]*=[ \\t]*")([^"]*)(")`);

function readTableKey(toml, header, key, label) {
	const match = toml.match(tableKey(header, key));
	if (!match) throw new Error(`${label}: [${header}] ${key} non trovato`);
	return match[2];
}

export const readWorkspaceVersion = (cargoToml) => readTableKey(cargoToml, 'workspace.package', 'version', CARGO_TOML);

/** Paths of the workspace members listed in the root Cargo.toml. */
export function workspaceMembers(cargoToml) {
	const block = cargoToml.match(/^members\s*=\s*\[([^\]]*)\]/m);
	if (!block) throw new Error(`${CARGO_TOML}: [workspace] members non trovati`);
	return [...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
}

/** Package name of a member's Cargo.toml (`[package] name`). */
export const crateName = (memberToml, label) => readTableKey(memberToml, 'package', 'name', label);

/** Lockfile pattern of one crate's version (group 1 / 2 around the version). */
export const lockEntry = (name) => new RegExp(`(\\[\\[package\\]\\]\\r?\\nname = "${escapeRegExp(name)}"\\r?\\nversion = ")[^"]*(")`);

export function bumpCargoToml(raw, version) {
	readWorkspaceVersion(raw); // clear error when the table or the key is missing
	return raw.replace(tableKey('workspace.package', 'version'), `$1${version}$3`);
}

export function bumpCargoLock(raw, version, crates) {
	return crates.reduce((text, name) => replaceOnce(text, lockEntry(name), version, `${CARGO_LOCK} (${name})`), raw);
}

export function bumpPackageJson(raw, version) {
	const eol = eolOf(raw);
	const pkg = JSON.parse(raw);
	pkg.version = version;
	// Tabs: the indentation of apps/desktop/package.json.
	return JSON.stringify(pkg, null, '\t').replace(/\n/g, eol) + (raw.endsWith('\n') ? eol : '');
}

/** Names of every workspace crate, read from the members' Cargo.toml. */
export function workspaceCrates(root = ROOT) {
	const cargoToml = readFileSync(path.join(root, CARGO_TOML), 'utf8');
	return workspaceMembers(cargoToml).map((member) => {
		const file = path.join(member, 'Cargo.toml');
		return crateName(readFileSync(path.join(root, file), 'utf8'), file);
	});
}

// Computes every new content before writing, so an error never leaves files half updated.
export function bumpVersion(version, root = ROOT) {
	assertValidVersion(version);
	const crates = workspaceCrates(root);
	const bumps = [
		[CARGO_TOML, (raw) => bumpCargoToml(raw, version)],
		[CARGO_LOCK, (raw) => bumpCargoLock(raw, version, crates)],
		[PACKAGE_JSON, (raw) => bumpPackageJson(raw, version)]
	];
	const updates = bumps.map(([file, bump]) => {
		const target = path.join(root, file);
		const before = readFileSync(target, 'utf8');
		return { file, target, before, after: bump(before) };
	});
	const changed = updates.filter((u) => u.after !== u.before);
	for (const { target, after } of changed) writeFileSync(target, after);
	return changed.map((u) => u.file);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
	try {
		const changed = bumpVersion(process.argv[2]);
		console.log(changed.length ? `Aggiornati: ${changed.join(', ')}` : 'Nessuna modifica: versione già allineata.');
	} catch (err) {
		console.error(err.message);
		process.exitCode = 1;
	}
}
