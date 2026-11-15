---
name: worktrees
description: One Lampo environment per git worktree, so several coding agents or branches run side by side, each with its own URL, port and optionally its own MySQL database (shared, fresh or cloned): `lampo worktree up/down/list/prune`. Use when the user asks to "create an environment for branch X", "work on two branches in parallel", "give this worktree its own database / a cloned database", "run several agents on the same project", "what is the URL of this worktree", or to clean up worktree environments.
---

# Worktree environments

Core rules: `lampo:lampo`. A worktree environment is a copy of a registered project's site for
one linked git worktree: same PHP, extensions, php.ini, Apache directives and services as the
parent, document root inside the worktree, its own loopback port (stable across restarts) and
a `<slug>.<project>.localhost` alias. It never touches the hosts file (no UAC), is served
while it exists (even when the parent is inactive), and is stored in Lampo's config, never in
the repository.

## Without an environment

In a git worktree of a registered project, the shims and `agent context` already work, but
with the **parent's URL and database** (`resolution.kind` is `worktree-without-environment` and
`hints` says so). Two agents migrating the same database will collide: give each worktree its
own environment before running migrations or tests.

## Create one (allowed without asking)

1. Create the git worktree, if it does not exist yet (Lampo does not create it):

   ```powershell
   git worktree add ..\shop-fix-login -b fix-login
   ```

2. Bring up its environment, from inside the worktree or with its path. Idempotent: running it
   again returns the existing environment, so it is safe at the start of every agent session.

   ```powershell
   cd ..\shop-fix-login
   lampo worktree up
   lampo worktree up --db fresh
   lampo worktree up C:\src\shop-fix-login --db clone:shop
   ```

   `--parent <project-id>` is needed only when the parent cannot be found from the worktree's
   main repository.

3. Read its URL and database (also printed by `worktree up --json` under `environment`):

   ```powershell
   lampo agent context --json
   ```

   `url` is the canonical `http://127.0.0.1:<port>`; `urls` adds
   `http://<slug>.<project>.localhost:<port>`. The site id is `<project>@<slug>`, the slug comes
   from the branch (`fix-login`), and it works with `--project shop@fix-login` or
   `LAMPO_PROJECT=shop@fix-login`.

### Database modes

| `--db` | What the environment uses | Typical use |
|--------|---------------------------|-------------|
| `shared` (default) | the parent's database | UI or code changes without migrations |
| `fresh` | new empty database `<parent db>_wt_<slug>` | run migrations and seeders from scratch |
| `clone:<database>` | new database `<parent db>_wt_<slug>`, a copy of `<database>` | realistic data, destructive tests, risky migrations |

The clone source must be a database on the parent's MySQL instance, usually the parent's own
(`database.database` in the parent's `agent context`, e.g. `shop`). The mode is chosen at
creation: to change it, remove the environment (see below) and bring it up again.

After `fresh` or `clone:`, point the worktree's `.env` at the new database (allowed): copy
`DB_DATABASE` (and `APP_URL` = the environment `url`) from `agent context`. Worktrees usually
have their own untracked `.env`; create it from `.env.example` or the main checkout's `.env`
if it is missing, then `php artisan config:clear` for Laravel.

## List

```powershell
lampo worktree list
lampo worktree list --project shop
```

`stale: true` marks an environment whose folder no longer exists.

## Remove

```powershell
lampo worktree down
lampo worktree down shop@fix-login
lampo worktree down C:\src\shop-fix-login --drop-database
```

`down` removes the environment (the folder and branch stay). Its own database is kept unless
`--drop-database` is given. ASK the user before removing an environment that owns a database,
and before `--drop-database`; a shared-mode environment can be removed when the user's task
says the worktree is finished. Remove the git worktree itself with `git worktree remove` only
when the user asks.

### Prune

```powershell
lampo worktree prune --dry-run
lampo worktree prune
lampo worktree prune --drop-databases
```

`prune` removes the environments whose folder is gone. Run `--dry-run` first and show the list;
pruning environments that own databases, and `--drop-databases` in particular, needs the
user's yes.

## Recommended multi-agent workflow

1. The main checkout stays the user's: agents do not run migrations against its database.
2. One git worktree and one branch per agent or task.
3. In each worktree, at the start of the session:
   `lampo worktree up --db clone:<parent db>` (or `--db fresh` when seeders are enough), then
   `lampo agent context --json` and update that worktree's `.env` (`DB_DATABASE`, `APP_URL`).
4. Run `composer install` and `npm install` inside the worktree (vendor folders are per
   checkout); build front-end assets there too.
5. Test against the environment's own URL; each agent has its own port, so they never collide.
6. When the work is merged: tell the user which environments and databases exist and, with
   their approval, `lampo worktree down <site> --drop-database`, then `git worktree remove`.

## Troubleshooting

- `worktree up` exit 4 "not a linked git worktree": the folder is the main checkout or not a
  git worktree; create one with `git worktree add`.
- Exit 3 or 6: the parent project is not registered; register the main checkout first
  (`lampo:new-project`) or pass `--parent <id>`.
- The URL answers with the parent's code: the request went to the parent's address; use the
  worktree `url` from `agent context`.
- Port range exhausted (exit 5): prune stale environments (with approval) or, with the user's
  yes, widen the range: `lampo settings` shows it, `lampo settings set --loopback-range 8100-8299`
  changes it (existing environments keep their ports).
