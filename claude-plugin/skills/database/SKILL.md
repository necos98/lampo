---
name: database
description: MySQL databases of a Lampo project: connection details, `lampo db list/create/import/export/drop`, the mysql and mysqldump shims, writing DB_* values into the app's .env, worktree database modes (shared, fresh, clone) and the safety rules for destructive operations. Use when the user asks "what are the database credentials", "create a database", "import this dump/.sql file", "export/backup the database", "reset/drop the database", "connect with mysql", "the app cannot connect to MySQL", or needs a separate database for tests or a worktree.
---

# Databases on Lampo

Core rules (autonomy, exit codes): `lampo:lampo`. Lampo runs shared MySQL instances, one per
series (8.4 by default, 5.7 for legacy projects; MariaDB and PostgreSQL are not available).
Every project that uses MySQL gets its own database, named after the project id (`my-shop` ->
`my_shop`), created when the project is registered.

## Connection details

```powershell
lampo agent context --json
```

`database` holds `mysqlVersion`, `host` (127.0.0.1), `port` (3306 for 8.4 by default; another
series has its own port, see `services`), `user` (`root`), `password` (empty) and `database`.
Inside a worktree environment with its own database these are that database's values. Use
`127.0.0.1`, not `localhost` (on Windows `localhost` may resolve to IPv6 first).

The `mysql` and `mysqldump` shims already connect to the site's instance:

```powershell
mysql -e "SHOW TABLES" my_shop
mysqldump --single-transaction --result-file=backup.sql my_shop
```

Prefer `--result-file` (or `lampo db export`) over `>`: in Windows PowerShell 5.1 a `>`
redirection re-encodes the dump as UTF-16, which MySQL cannot import. For the same reason feed
dumps with `lampo db import` (an import: ask first, see below), not `Get-Content | mysql`.

## Writing `.env`

Lampo never edits `.env`. You may write the values it reports into the app's `.env` (keep
every other line as it is). For Laravel and most PHP frameworks:

```dotenv
DB_CONNECTION=mysql
DB_HOST=127.0.0.1
DB_PORT=3306
DB_DATABASE=my_shop
DB_USERNAME=root
DB_PASSWORD=
```

WordPress: `DB_NAME`, `DB_USER`, `DB_PASSWORD`, `DB_HOST` (`127.0.0.1:3306`) in
`wp-config.php`. Symfony: `DATABASE_URL="mysql://root:@127.0.0.1:3306/my_shop?serverVersion=8.4"`.
After editing a Laravel `.env`, run `php artisan config:clear` (a cached config ignores `.env`).

## Commands

All `lampo db` commands act on the resolved site's MySQL instance and go through the app
(started headless if needed). The default database is the site's own.

```powershell
lampo db list
lampo db create
lampo db create my_shop_testing
lampo db export backup.sql
lampo db export C:\backups\shop.sql my_shop
```

`db export` writes a complete dump (atomically: a partial file is never left behind). It is
read-only for the database and allowed without asking; prefer it before any risky change.

### Import: always ask first

`lampo db import <file> [<name>]` creates the database if it is missing and then replays the
dump into it: tables in the dump are dropped and recreated, so it **overwrites** data. Always
ask the user before an import, naming the target database, and offer an export first:

```powershell
lampo db export before-import.sql
lampo db import dump.sql
lampo db import C:\dumps\legacy.sql legacy_copy
```

Dump caveats:

- A dump made with `--databases` / `--all-databases` contains `CREATE DATABASE` and `USE`
  lines that send the data to the original name, whatever target you pass. Check the head of
  the file (`Select-String -Path dump.sql -Pattern '^USE ', '^CREATE DATABASE' | Select -First 5`)
  and, with the user's approval, import into that name or dump again without those options.
- Dumps from MariaDB may use syntax MySQL rejects (see `lampo:migrate`, "MariaDB").
- Compressed dumps (`.sql.gz`, `.zip`) must be extracted first (`tar -xf` or
  `Expand-Archive`); `lampo db import` reads plain SQL.
- Exit code 12 means the mysql client failed: the message has the SQL error and its line.

### Drop: always ask first

```powershell
lampo db drop my_shop_old --yes
```

`--yes` is required because `lampo` never prompts: it is not consent. Ask the user, name the
database, and only then run it. Never drop the project's main database to "fix" something.
Deleting a Lampo project never drops its database.

## A separate database for tests

Create it once (allowed), then point the test environment at it, for example
`DB_DATABASE=my_shop_testing` in `.env.testing` or `phpunit.xml` (see `lampo:laravel`):

```powershell
lampo db create my_shop_testing
```

Never run tests that wipe the database (`RefreshDatabase`, `migrate:fresh`) against the main
development database unless the user agrees.

## Worktree database modes

`lampo worktree up --db <mode>` chooses how a worktree environment gets its database (details
in `lampo:worktrees`):

| Mode | Database | Use it when |
|------|----------|-------------|
| `shared` (default) | the parent project's database | read-only work, no migrations |
| `fresh` | a new empty `<db>_wt_<slug>` | migrations from scratch, seeders, destructive tests |
| `clone:<database>` | a new `<db>_wt_<slug>` copied from `<database>` | realistic data without touching the original |

Creating `fresh` or `clone:` databases is allowed without asking. Dropping them
(`lampo worktree down --drop-database`, `lampo worktree prune --drop-databases`) needs the
user's yes.

## Troubleshooting

- `SQLSTATE[HY000] [2002]` / connection refused: MySQL is not running or the port differs.
  `lampo status`, then `lampo up` (starts the planned services) or `lampo up mysql-8.4`; read
  the port from `agent context` again.
- `Access denied for user`: the app uses other credentials; set `root` with an empty password
  as above (or ask the user if they created their own MySQL users).
- `Unknown database`: `lampo db list`, then `lampo db create <name>` (allowed).
- MySQL does not start: `lampo logs mysql-8.4-error -n 80` and `lampo:services-logs`.
- A `lampo db` command fails with exit 4 "uses no MySQL": the project has no MySQL service; it is
  enabled in the Lampo app, "Progetti" > project > "Database e cache".
