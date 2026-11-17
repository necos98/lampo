import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { CheckStatus, DoctorCheck, VcRuntimeIssue } from '../api/index.js';
import { fixActions } from './fixes.ts';

const check = (id: string, status: CheckStatus = 'warning'): DoctorCheck => ({ id, status, message: 'detail', fix: 'hint' });
const noVc = { missingVcRuntimes: [] as VcRuntimeIssue[] };

test('a passing check offers nothing', () => {
	for (const id of ['shims-on-path', 'stale-worktrees', 'hosts-block', 'services', 'packages', 'folders', 'mysql-auth', 'defender', 'ports', 'vc-runtime']) {
		assert.deepEqual(fixActions(check(id, 'ok'), noVc), [], id);
	}
});

test('fixes Lampo can run itself are commands', () => {
	assert.deepEqual(fixActions(check('shims-on-path'), noVc), [{ kind: 'command', command: 'install-shims' }]);
	assert.deepEqual(fixActions(check('stale-worktrees'), noVc), [{ kind: 'command', command: 'prune-worktrees' }]);
	assert.deepEqual(fixActions(check('hosts-block'), noVc), [{ kind: 'command', command: 'apply-hosts' }]);
	assert.deepEqual(fixActions(check('services', 'error'), noVc), [{ kind: 'command', command: 'start-services' }]);
});

test('fixes made on another page navigate there', () => {
	assert.deepEqual(fixActions(check('packages', 'error'), noVc), [{ kind: 'navigate', href: '/packages', page: 'packages' }]);
	assert.deepEqual(fixActions(check('folders'), noVc), [{ kind: 'navigate', href: '/projects', page: 'projects' }]);
	assert.deepEqual(fixActions(check('mysql-auth'), noVc), [{ kind: 'navigate', href: '/projects', page: 'projects' }]);
	assert.deepEqual(fixActions(check('defender'), noVc), [{ kind: 'navigate', href: '/settings', page: 'settings' }]);
	assert.deepEqual(fixActions(check('ports', 'error'), noVc), [{ kind: 'navigate', href: '/services', page: 'services' }]);
});

test('each missing Visual C++ runtime gets its download link', () => {
	const missingVcRuntimes: VcRuntimeIssue[] = [
		{ redistributable: 'Visual C++ 2012 (x64)', downloadUrl: 'https://example.test/2012', requiredBy: ['php-5.6.40'] },
		{ redistributable: 'Visual C++ 2015-2022 (x64)', downloadUrl: 'https://example.test/2022', requiredBy: ['apache'] }
	];
	assert.deepEqual(fixActions(check('vc-runtime', 'error'), { missingVcRuntimes }), [
		{ kind: 'download', href: 'https://example.test/2012', label: 'Visual C++ 2012 (x64)' },
		{ kind: 'download', href: 'https://example.test/2022', label: 'Visual C++ 2015-2022 (x64)' }
	]);
});

test('checks fixed only outside Lampo, or unknown, offer nothing', () => {
	for (const id of ['path-shadowing', 'app', 'a-future-check']) assert.deepEqual(fixActions(check(id), noVc), [], id);
});
