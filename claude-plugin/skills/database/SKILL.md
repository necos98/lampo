---
name: database
description: MySQL databases of a Lampo project: connection details, `lampo db list/create/import/export/drop`, running SQL as an agent with the mysql and mysqldump shims (default database, non-interactive `-e`, parseable output, SQL scripts), writing DB_* values into the app's .env, worktree database modes (shared, fresh, clone) and the safety rules for destructive operations. Use when the user asks "what are the database credentials", "create a database", "import this dump/.sql/.sql.gz/.zip file", "export/backup the database", "reset/drop the database", "connect with mysql", "run this query", "show the tables", "the app cannot connect to MySQL", or needs a separate database for tests or a worktree.
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

## The `mysql` client

The `mysql` shim runs the site's client with `--host=127.0.0.1 --port=<the series' port>
--user=root --default-character-set=utf8mb4` and **selects the site's database** (in a worktree
environment with its own database, that one). Your options come after these defaults, so they
win. Outside any project (the default PHP) no database is selected.

```powershell
mysql -e "SHOW TABLES"
mysql -N -B -e "SELECT id, email FROM users ORDER BY id LIMIT 5"
mysql --vertical -e "SHOW CREATE TABLE users"
mysql --table -e "SELECT status, COUNT(*) FROM orders GROUP BY status"
mysql -D my_shop_testing -e "SHOW TABLES"
mysql --database= -e "SHOW DATABASES"
```

- **Always non-interactive**: pass the SQL with `-e "..."` (several statements separated by
  `;`; execution stops at the first error). Never run bare `mysql`: it opens a prompt (or waits
  on stdin) and the command never returns.
- **Parseable output**: `-N -B` (`--skip-column-names --batch`) prints one row per line,
  columns separated by tabs, `NULL` for null, tabs/newlines/backslashes escaped as `\t`, `\n`,
  `\\` (add `-r` to print them raw). Without `-N` the first line is the header. `--vertical`
  (`-E`) prints one `column: value` line per field, best for wide rows; `--table` (`-t`) an
  aligned grid to show the user. Output is UTF-8.
- **Another database**: `-D <name>`, `--database=<name>` or a name as the last argument. **No
  database** (`SHOW DATABASES`, `CREATE DATABASE`, the site's database does not exist yet):
  `--database=` with nothing after `=`. Do not write `-D ""`: Windows PowerShell drops the
  empty argument and `-D` then swallows the next one. To create a database prefer
  `lampo db create <name>`.
- **SQL scripts**: `mysql -e "source C:/path/to/script.sql"` (forward slashes; it runs against
  the selected database and stops at the first error). PowerShell has no `<` redirection and
  `Get-Content script.sql | mysql` re-encodes the text: use `source`. Dumps (backups, exports
  from another machine, `.sql.gz`, `.zip`) are imports: `lampo db import` and ask first (below).
- **Exit codes**: 0 success; 1 SQL or connection error (`ERROR 1146 (42S02) at line 1: ...` on
  stderr, nothing on stdout); 2 bad command line; 127 the shim could not resolve the site or
  the client (`lampo agent context --json`, read `hints`). `ERROR 1049 Unknown database` means
  the site's database is missing: `lampo db list`, then `lampo db create` (allowed).
- Keep host and user as they are: `127.0.0.1`, not `localhost` (on Windows it may resolve to
  IPv6 first), and `root` with no password (do not pass `-p`, it would prompt).
- Reading is allowed. Statements that destroy data on the project's main database (`DROP`,
  `TRUNCATE`, `DELETE`/`UPDATE` of many rows, destructive `ALTER`) need the user's yes, like
  `lampo db drop`; export first (`lampo db export before-change.sql`). A `fresh`/`clone:`
  worktree database is the agent's own.
- `lampo exec -- mysql -e "SHOW TABLES"` and a shell loaded with `lampo env` behave the same:
  the shim directory comes first on their PATH and `LAMPO_PROJECT` pins the site. Scripts that
  need the values themselves read `LAMPO_DB_HOST`, `LAMPO_DB_PORT`, `LAMPO_DB_USER` and
  `LAMPO_DB_DATABASE` from `lampo env`.

## `mysqldump`

With no database named (no positional argument, no `--databases`/`--all-databases`) the shim
dumps the site's database; names you pass are used as they are.

```powershell
mysqldump --single-transaction --result-file=backup.sql
mysqldump --no-data --result-file=schema.sql
mysqldump --single-transaction --result-file=orders.sql my_shop orders customers
```

For a backup prefer `lampo db export`. Prefer `--result-file` over `>`: in Windows PowerShell
5.1 a `>` redirection re-encodes the dump as UTF-16, which MySQL cannot import. For the same
reason feed dumps with `lampo db import` (an import: ask first, see below), not
`Get-Content | mysql`.

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
lampo db export C:\backups\shop.sql.gz my_shop
```

`db export` writes a complete dump (atomically: a partial file is never left behind); a file
name ending in `.gz` is written gzip-compressed (`.zip` is refused). It is read-only for the
database and allowed without asking; prefer it before any risky change.

### Import: always ask first

`lampo db import <file> [<name>]` creates the database if it is missing and then replays the
dump into it: tables in the dump are dropped and recreated, so it **overwrites** data. Always
ask the user before an import, naming the target database, and offer an export first:

```powershell
lampo db export before-import.sql
lampo db import dump.sql
lampo db import C:\dumps\legacy.sql legacy_copy
lampo db import C:\dumps\shop.sql.gz
lampo db import C:\dumps\backup.zip shop_copy
```

Compressed dumps import as they are, never extract them first: gzip (`.sql.gz`, `.gz`) and a
`.zip` that holds exactly one `.sql` file are recognised from their content and decompressed on
the fly; the progress counts bytes of the compressed file. A zip with no `.sql` or with several
(`contains 2 .sql files`, exit 4) must be split: extract it (`Expand-Archive`) and import the
files one at a time, in the order the user confirms. Tar archives (`.tar.gz`, `.tgz`) and other
compressors (`.bz2`, `.xz`, `.zst`, `.7z`, `.rar`) are refused: extract the `.sql` first
(`tar -xf` handles `.tar.gz`). A corrupt or truncated archive stops the import with exit 4
(`corrupt or truncated gzip data`).

A dump whose first 64 KB contain `USE` or `CREATE DATABASE` (made with `mysqldump --databases`
or `--all-databases`) is refused with exit 4 (`switches or creates a database`) before anything
is written: its statements would run in the database it names, not in the target. Tell the
user which line was found and which database it names; prefer removing those statements (or a
new export without `--databases`). Add `--allow-use` only when the user confirms that writing
to that database is intended.

An import that fails or is cancelled part-way leaves what already ran: the error says the
database `may now be incomplete`. Report it, never retry on your own; with the user's
confirmation, drop the database (`lampo db drop <name> --yes`) and import again, or import the
backup exported before.

Dump caveats:

- A dump made with `--databases` / `--all-databases` contains `CREATE DATABASE` and `USE`
  lines that send the data to the original name, whatever target you pass. Check the head of
  the file (`Select-String -Path dump.sql -Pattern '^USE ', '^CREATE DATABASE' | Select -First 5`;
  for a compressed dump ask the user, or look inside a `.zip` with `Expand-Archive` into a
  temporary folder) and, with the user's approval, import into that name or dump again without
  those options.
- Dumps from MariaDB may use syntax MySQL rejects (see `lampo:migrate`, "MariaDB").
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
- A `lampo db` command fails with exit 4 "uses no MySQL": the project has no MySQL service.
  Enabling it changes the project, so ask first; then `lampo project set --service mysql` (the
  default series; `mysql@5.7` for another configured one) and `lampo db create`.
- The project should point at another existing database: `lampo project set --database <name>`
  (ask first on a main project). It only changes the name Lampo reports (`agent context`,
  `LAMPO_DB_*`); it never creates, renames or copies a database.
