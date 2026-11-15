---
name: lampo
description: Use when working in a PHP project served by Lampo (local PHP environments for Windows), or when the user mentions Lampo, its local URL, database, PHP version, services or git worktree environments. Explains how to get the project's context with the lampo CLI.
---

# Lampo

Lampo serves this project locally (Apache, PHP, MySQL, Redis) and provides the `lampo` CLI.
`lampo` is on PATH; if it is not, use `%LOCALAPPDATA%\Lampo\bin\lampo.exe`.

## First step

Run this before anything that depends on the environment (URLs, database, PHP version):

```powershell
lampo agent context --json
```

It prints, in one JSON document: the resolved project or git worktree, its URL(s), document
root, PHP version and binary, the database connection, the services it uses, the log files,
every CLI command, the exit codes and actionable `hints`. It is read-only and works even when
the Lampo app is not running.

## Rules

- `php`, `composer`, `mysql`, `mysqldump` and `redis-cli` on PATH are Lampo shims that already
  use this project's versions: call them directly.
- Add `--json` to any `lampo` command (except `exec`) for machine-readable output; branch on
  the documented exit codes.
- `lampo` never prompts. Do not pass `--allow-elevation` unless the user asked for it.
- Lampo never writes credentials into `.env`: copy them from `agent context` when needed.

More specific skills (databases, worktrees, logs, troubleshooting) will be added to this plugin.
