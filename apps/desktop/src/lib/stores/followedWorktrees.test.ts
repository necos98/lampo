import assert from 'node:assert/strict';
import { test } from 'node:test';

import { FollowedWorktrees } from './followedWorktrees.ts';

const A = 'C:\\wt\\a';
const B = 'C:\\wt\\b';
const C = 'C:\\wt\\c';

test('an unknown project follows nothing', () => {
	assert.deepEqual(new FollowedWorktrees().forProject('shop'), []);
});

test('folders are kept per project, in the order they were started', () => {
	const followed = new FollowedWorktrees();
	followed.follow('shop', A);
	followed.follow('blog', B);
	followed.follow('shop', C);

	assert.deepEqual(followed.forProject('shop'), [A, C]);
	assert.deepEqual(followed.forProject('blog'), [B]);
});

test('following the same folder twice keeps one entry', () => {
	const followed = new FollowedWorktrees();
	followed.follow('shop', A);
	followed.follow('shop', A);
	assert.deepEqual(followed.forProject('shop'), [A]);
});

test('unfollow removes only that folder of that project', () => {
	const followed = new FollowedWorktrees();
	followed.follow('shop', A);
	followed.follow('shop', B);
	followed.follow('blog', A);

	followed.unfollow('shop', A);

	assert.deepEqual(followed.forProject('shop'), [B]);
	assert.deepEqual(followed.forProject('blog'), [A]);
});

test('unfollowing an unknown folder or project changes nothing', () => {
	const followed = new FollowedWorktrees();
	followed.follow('shop', A);
	followed.unfollow('shop', C);
	followed.unfollow('blog', A);
	assert.deepEqual(followed.forProject('shop'), [A]);
});

test('the state survives the object reading it: a new instance on the same record sees it', () => {
	const byProject: Record<string, string[]> = {};
	new FollowedWorktrees(byProject).follow('shop', A);
	assert.deepEqual(new FollowedWorktrees(byProject).forProject('shop'), [A]);
});

test('the last unfollow drops the project from the record it was given', () => {
	const byProject: Record<string, string[]> = {};
	const followed = new FollowedWorktrees(byProject);
	followed.follow('shop', A);
	assert.deepEqual(byProject, { shop: [A] });

	followed.unfollow('shop', A);
	assert.deepEqual(byProject, {});
});
