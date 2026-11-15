# From Laravel Herd for Windows

Part of `lampo:migrate`. Herd serves sites with nginx and its own PHP builds; databases and
other services come from Herd Pro (or from separate tools). Paths below are Herd's usual ones
on Windows; verify them on the machine (`herd --help` lists the commands of the installed
version). Read only: do not run commands that change Herd (`herd isolate`, `herd unlink`,
`herd park`, `herd secure`...).

## Inventory

| What | Where (typically) |
|------|-------------------|
| Sites | every folder in a parked path, by default `%USERPROFILE%\Herd\<site>`, served as `http://<site>.test`; linked sites can live anywhere |
| Config | `%USERPROFILE%\.config\herd\` (for example `config\valet\config.json` with the TLD and the parked `paths`, plus per-site nginx config under `config\valet\`) |
| PHP binaries | Herd's `bin` folder under `%USERPROFILE%\.config\herd\` (added to PATH by Herd) |
| Per-site PHP | set with `herd isolate <version>`; read it with the commands below |
| Databases (Herd Pro) | MySQL, PostgreSQL, Redis... configured in Herd's "Services" window, each with its own port |
| Mail, dumps (Herd Pro) | Herd's mail and dump tools: not provided by Lampo |

Read-only commands (run them in the site folder):

```powershell
herd parked
herd links
herd isolated
herd which-php
php -v
Get-Content "$env:USERPROFILE\.config\herd\config\valet\config.json"
```

If a command does not exist in the installed version, look in Herd's UI ("Sites": PHP version
and URL per site; "Services": database engine, version and port). From the project: `.env`
(`DB_CONNECTION`, `DB_HOST`, `DB_PORT`, `DB_USERNAME`, `DB_PASSWORD`, `APP_URL=http://shop.test`).

Check `DB_CONNECTION`: `pgsql` means PostgreSQL, **not supported** (SKILL.md, step 2). `sqlite`
needs no database migration: the file stays in the project (`database\database.sqlite`).

## Register in place

```powershell
lampo project create $HOME\Herd\shop --php 8.3 --activate
```

Herd sites are already Laravel-ready (`public` document root); nginx-specific rules from a
custom Herd/Valet driver or nginx config must become `.htaccess` rules or Apache directives
(`lampo:services-logs`). Keep `.test` for Herd: Lampo uses `http://shop.localhost` and its port.
`herd secure` sites were HTTPS: on Lampo use `http://` (set `APP_URL` accordingly).

## Export the database (MySQL in Herd Pro, or another MySQL)

With the service running, dump it through Herd's port (from the Services window or `.env`).
Lampo's `mysqldump` shim puts Lampo's host, port and user first and the arguments you pass
after them, so explicit `-h`/`-P`/`-u` win and it reaches Herd's server. Make sure the port is
Herd's, not Lampo's: if both use 3306, only one of them can be running.

```powershell
New-Item -ItemType Directory -Force $HOME\lampo-migration\shop
mysqldump -h 127.0.0.1 -P 3306 -u root --single-transaction --routines --triggers --result-file="$HOME\lampo-migration\shop\shop.sql" shop
```

Use the user and password from `.env` (Herd Pro's MySQL usually `root` with an empty password).
Do this before stopping Herd's services. Then follow SKILL.md step 4.

## Ports and stopping

Herd's nginx listens on 80 (and 443); Herd Pro services on their own ports (MySQL 3306 by
default, Redis 6379). With the user's yes, they stop Herd's services and its web server from the
Herd tray menu or window ("Stop all"), or quit Herd. Do not stop them yourself.

## PATH

Herd puts its own `php`, `composer` and `laravel` first on PATH: they shadow Lampo's shims, so
`php -v` shows Herd's PHP. Check with `where.exe php` and `lampo doctor` (`path-shadowing`).
Tell the user: the order of PATH entries decides which `php` wins; do not edit PATH yourself.
Meanwhile `lampo exec -- php -v` always uses Lampo's PHP.

## Rollback

Restore `.env` from the backup folder, start Herd's services again, deactivate the Lampo
project. Herd's sites, isolation settings and databases were never changed.
