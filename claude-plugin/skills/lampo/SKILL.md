---
name: lampo
description: Core guide to Lampo, the local PHP environment for Windows (Apache, any PHP version, MySQL, Redis, Mailpit for caught mail, per-git-worktree environments) and its `lampo` CLI. Use whenever the project is served by Lampo or the user mentions Lampo, its local URL (*.localhost, 127.0.0.1:81xx), its PHP version, database, services or shims, or asks "what is my local URL", "which PHP does this project use", "is Lampo running", "why did the lampo command fail" (exit codes). Explains how to orient with `lampo agent context --json`, site resolution, JSON output and exit codes, what the agent may do alone and what needs the user's approval, and which specialised `lampo:` skill to load next.
---

# Lampo

Lampo is a desktop app plus a CLI that serves PHP projects locally on Windows, without Docker
and without Windows services: one Apache (mod_fcgid) with a virtual host per project, any PHP
from 5.6 up chosen per project, shared MySQL instances (one per series: 8.4, 5.7), optional
Redis, a shared Mailpit that catches the sites' mail (SMTP 1025, web UI http://127.0.0.1:8025),
and a separate environment for every git worktree. Lampo owns these processes; you drive
it with `lampo`. If `lampo` is not on PATH, use `%LOCALAPPDATA%\Lampo\bin\lampo.exe`.

Lampo is a "ghost" integration: it never writes files into the user's project for its own sake.
Never create `AGENTS.md`, `CLAUDE.md`, `.claude/settings.json` or any Lampo config file in the
project, and never install anything into the project just for Lampo. Editing the app's own
`.env` (database, Redis, mail, `APP_URL`) is normal project work and is fine.

## 1. Orient first

At session start a hook may already have added a "Lampo project context" block (project, URL,
PHP, database, services, logs). It is a snapshot. Before anything that depends on the
environment, get the current state:

```powershell
lampo agent context --json
```

One read-only JSON document (works when the app is not running): `resolution` (how the site was
found), `projectId`, `root`, `worktree`, `url` / `urls`, `documentRoot`, `php` (version, binary,
ini dir, Composer), `database` (host, port, user, password, database), `mail` (Mailpit: host,
`smtpPort`, `webUrl`, when the site uses it), `services` (with ports),
`logs` (source ids and files), `commands`, `cli.exitCodes`, `hints`. Read the `hints`: they say
what to run next (for example "project is not active", "no worktree environment is up").

Other read-only views: `lampo status` (services and projects), `lampo project list`,
`lampo project info`, `lampo project detect` (what Lampo would detect in a folder, without
registering it), `lampo worktree list`, `lampo logs`, `lampo doctor`, `lampo env`.

## 2. Which site a command acts on

Resolution order, for both the CLI and the shims:

1. `--project <site>` (global flag: `shop` or `shop@feature-x`);
2. the `LAMPO_PROJECT` environment variable (same syntax), set by `lampo exec`, `lampo env` and
   the app's terminal together with `LAMPO_PROJECT_ROOT`: it holds inside that folder or
   outside every project, while the folder of another project wins (one `LAMPO_PROJECT=...
   ignored` line on stderr; `LAMPO_QUIET=1` hides it);
3. the current folder: the longest match among project folders and worktree-environment
   folders;
4. a git worktree of a registered project (read from `.git`): without its own environment it
   uses the parent project's URL and database, and `agent context` says so in `hints`;
5. otherwise the global default PHP (no project: `projectId` is null).

So `cd` into the project (or pass `--project`) before running commands. `resolution.kind` in
`agent context` tells you which case applied.

## 3. Output and exit codes

- Add `--json` to any command except `exec`: exactly one JSON document on stdout, diagnostics
  on stderr. Errors in JSON mode are `{"error": {"code", "message", "hint", "exitCode",
  "details"?}}`; `hint` is always an actionable sentence.
- `lampo` never prompts and never blocks on input. Destructive commands need an explicit flag
  (`--yes`, `--drop-database`). Nothing triggers UAC without `--allow-elevation`.
- Read-only commands run in-process. Commands that change state go to the Lampo app, which is
  started headless when needed (`--no-start` turns that into exit code 9).

| Exit | Meaning | What to do |
|-----:|---------|------------|
| 0 | success | Continue. |
| 1 | unclassified failure | Read `message`; run `lampo doctor`; report it if unclear. |
| 2 | invalid command line | Fix the syntax with `--help` on the command (`lampo db import --help`). Never guess flags. |
| 3 | not found | Check names with `lampo project list`, `lampo worktree list`, `lampo package list`, `lampo logs`. |
| 4 | invalid input | Fix the value (PHP minor like `8.3`, a valid database name...). |
| 5 | conflict: duplicate, port in use, in use | Inspect `lampo status`; pick another id/name; for ports see `lampo:services-logs`. |
| 6 | no project for the current directory | Check the cwd or pass `--project`; if the folder should be a Lampo project, offer `lampo project create` (see `lampo:new-project`) and ask first. |
| 7 | needs the hosts-file UAC prompt | Ask the user. Only on a yes, re-run with `--allow-elevation`; otherwise keep `.localhost` addressing. |
| 8 | UAC prompt refused | Do not retry; tell the user and stay on `.localhost` addresses. |
| 9 | app not running and could not be started | Ask the user to open the Lampo app; check you did not pass `--no-start`; `lampo doctor`. |
| 10 | required package not installed | Install what the hint names, e.g. `lampo package install php 8.3` or `lampo package install mysql 8.4`, then retry. |
| 11 | network or checksum failure | Retry once; then report (do not work around a checksum failure). |
| 12 | external program failed | Read `details.output` and the logs: `lampo logs apache-error`, `lampo logs php`, the MySQL error log. |
| 13 | cancelled or timed out | Retry once; check `lampo status`. |
| 14 | `doctor` found problems | Read every `[error]` line and its `fix:`; act on the ones within your autonomy. |
| 15 | CLI/app version mismatch | Ask the user to update Lampo and restart the app; do not work around it. |

Shims return the tool's own exit code, or 127 when they cannot resolve a PHP or the tool.

## 4. Autonomy rules

You MAY, without asking:

- run any read-only `lampo` command (`agent context`, `status`, `project list`, `project info`,
  `project detect`, `worktree list`, `logs`, `doctor`, `env`, `package list`, `package extensions`, `db list`, `db export`, `shims status`);
- start or restart services: `lampo up`, `lampo restart`;
- read the global settings (ports, default PHP): `lampo settings`;
- create databases (`lampo db create`) and worktree environments (`lampo worktree up` with any
  `--db` mode);
- run `php`, `composer`, `php artisan`, `mysql`, `redis-cli` through the shims or `lampo exec`;
- write the database, Redis, mail and `APP_URL` values Lampo reports into the project's `.env`;
- install a package the current task needs when a command fails with exit code 10 (say so);
- enable what the task needs with `lampo project set`: a bundled extension (`--ext`), an add-on
  (`--addon xdebug`), a php.ini value (`--ini`), Redis (`--service redis`), Mailpit
  (`--service mailpit`); say what you changed.

You MUST ASK the user first, and wait for a clear yes, before:

- dropping, resetting or overwriting a database: `lampo db drop`, every `lampo db import`
  (it writes into an existing database), `php artisan migrate:fresh` / `db:wipe` /
  `migrate:reset` on a database that is not a throwaway one;
- using `--allow-elevation` (it opens a UAC prompt);
- changing ports or other global settings with `lampo settings set` (Apache, MySQL, Redis, Mailpit
  ports, default PHP, loopback range): they affect every project and restart running services;
- changing the PHP version of a main project (`lampo project set --php 8.4`, or
  `lampo project set-php 8.4 <id>`: version first), or with
  `lampo project set` removing a service, switching its MySQL series, or changing its database
  name, public folder or Apache directives;
- removing a project (`lampo project remove <id> --yes`; its folder and databases stay) or
  removing one of its addresses (`lampo project set --no-address <address>`);
- removing worktree environments that own a database (`lampo worktree down --drop-database`,
  `lampo worktree prune --drop-databases`, or `lampo worktree down` of an environment whose
  database the user may still want);
- uninstalling packages (`lampo package uninstall <kind> <version> --yes`; it refuses a build
  still in use, exit 5, and never touches projects or databases).

Also: `lampo down` and `lampo quit` stop services for every project, not just this one: only
when the user asks or the task really needs it, and say so. Never stop or uninstall software
that is not Lampo (Laragon, XAMPP, Docker, IIS) without the user's explicit approval.

## 5. Everyday facts

- `php`, `composer`, `mysql`, `mysqldump` and `redis-cli` on PATH are Lampo shims that pick
  this site's versions by folder: call them directly (`php artisan migrate`, `composer install`).
- Other tools with the site's toolchain on PATH: `lampo exec -- <command>`.
- URLs: `http://<id>.localhost` plus a dedicated port such as `http://127.0.0.1:8100`; worktree
  environments get their own port. Prefer the URL printed by `agent context`.
- MySQL: user `root`, empty password, host `127.0.0.1`, port of the series (3306 for 8.4 by
  default). The project database is named after the project id (`my-shop` -> `my_shop`).
  The `mysql` shim already selects it: `mysql -N -B -e "SHOW TABLES"`. Always pass SQL with
  `-e`, never open the interactive prompt (details: `lampo:database`).
- Lampo never edits `.env`; copy the values from `agent context` when the app needs them.
- Mail: projects using Mailpit (`--service mailpit`, on by default for new Laravel, Symfony,
  WordPress and generic PHP projects) send to SMTP `127.0.0.1:1025` (no auth, no TLS; PHP
  `mail()` already does through php.ini) and the mails show up at `http://127.0.0.1:8025`;
  nothing reaches real recipients. Read the ports from `mail` in `agent context`.
- A project's environment is edited with `lampo project set` (PHP version `--php`, extensions,
  add-ons, php.ini values, services, database name, public folder, Apache directives; see
  `lampo project set --help`). Only what you name changes; worktree environments follow the
  parent. Display name: `lampo project rename`; addresses: `lampo project set --address <address>` /
  `--no-address`; unregistering: `lampo project remove --yes`, with the user's yes (see
  `lampo:new-project`).
- Global settings: `lampo settings` shows them; with the user's yes,
  `lampo settings set --apache-port 8080`, `--mysql-port 8.4=3310`, `--redis-port 6380` or
  `--port mailpit=2025` (Mailpit's web UI: `--ui-port mailpit=9025`)
  moves a service (the port is checked first; running services restart on it). Then update
  `DB_PORT` / `REDIS_PORT` / `MAIL_PORT` / `APP_URL` in the `.env` files of the sites the output lists.
  The configured MySQL series themselves cannot be added or removed, only their ports.

## 6. Load the specialised skill

| Task | Skill |
|------|-------|
| PHP versions, extensions, Xdebug, php.ini, shims, `lampo exec` | `lampo:php` |
| Databases: create, import, export, drop, `.env`, worktree database modes | `lampo:database` |
| One environment per git worktree, parallel agents | `lampo:worktrees` |
| Services, logs, `lampo doctor`, 500/404 errors, ports, MySQL not starting, caught mail (Mailpit) | `lampo:services-logs` |
| Register a folder (preview detection first), clone a repository, new Laravel app, addresses, rename or remove a project | `lampo:new-project` |
| Laravel on Lampo: `.env`, migrations, queues, scheduler, tests, Vite | `lampo:laravel` |
| Move a project from Docker/Sail, Laragon, XAMPP/WAMP or Herd to Lampo | `lampo:migrate` |
