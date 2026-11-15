---
name: migrate
description: Move an existing PHP project to Lampo from another local environment: Docker or docker-compose (including Laravel Sail), Laragon, XAMPP, WampServer or Laravel Herd for Windows. Covers the inventory of the old setup, compatibility checks (PHP, MySQL vs MariaDB, PostgreSQL not supported, nginx rules, Mailpit/MailHog now provided by Lampo, search engines), registering the project, moving the database with the user's confirmation, updating .env, freeing ports 80/3306, verification, and a summary with rollback steps. Use when the user says "migrate/port/move this project to Lampo", "switch from Laragon/XAMPP/WAMP/Herd/Docker/Sail to Lampo", "import my Docker setup into Lampo", or wants to stop using their old local stack.
---

# Migrating a project to Lampo

Core rules: `lampo:lampo` (autonomy, exit codes). A migration is a sequence of small, checked
steps. Two rules hold throughout:

- **Never delete or modify the source environment**: no removing containers, volumes, Laragon
  or XAMPP folders, databases or virtual hosts; no editing their config. Stopping their
  services only with the user's yes (and only after the data has been exported).
- **The user confirms every data step**: show what will be imported where before any
  `lampo db import`; Lampo never edits `.env` on its own, you do, after backing it up.

Pick the reference for the source and keep it open while following the steps:
[docker.md](docker.md) (docker-compose, Dockerfile, Laravel Sail), [laragon.md](laragon.md),
[xampp-wamp.md](xampp-wamp.md) (XAMPP and WampServer), [herd.md](herd.md) (Herd for Windows).

## Step 1: inventory (read-only)

Collect, from the project and the source environment (the reference says where to look):

| Item | Where it usually is |
|------|---------------------|
| Project folder and document root | the source's vhost/site config; Laravel/Symfony: `public` |
| PHP version and extensions | image tag / Dockerfile, the source's PHP folder and `php.ini`, `composer.json` `require.php` and `ext-*` |
| Database engine, version, name, user, password, port | `.env` / `wp-config.php`, the compose file, the source's MySQL folder |
| Other services | Redis, Mailpit/MailHog, Meilisearch, Elasticsearch, queues, cron |
| Web server rules | `.htaccess` (works as is), nginx `location` blocks, Apache vhost directives |
| Hostname | `shop.test` and the like (the source's hosts entries) |

Then ask Lampo what it sees in the folder, without registering anything:

```powershell
lampo project detect C:\laragon\www\shop --json
```

It returns `projectType`, `presetId`, `phpVersion` (the minimum from `composer.json` or the
Docker image), `suggestedId`, `evidence`, `services` (from a compose file: `mysql`, `redis`,
`mailpit` for a Mailpit or MailHog container),
`warnings` (what Lampo cannot provide or replaces: PostgreSQL, MariaDB, MailHog, search engines) and
`alreadyRegistered` (an id: the folder is a Lampo project already, do not create it again).
Compare it with your inventory: where they differ, your inventory wins and you pass `--php` /
`--preset` in step 3. `lampo project list` and `lampo status` show what Lampo already knows.
Write the inventory down for the user in a short table before changing anything.

## Step 2: compatibility check

| Source has | On Lampo |
|------------|----------|
| PHP 5.6 to the newest | supported, per project (`lampo package install php <x.y>`) |
| MySQL 5.7 / 8.x | MySQL 8.4 (default) or 5.7; 8.0 dumps import into 8.4 |
| MariaDB | not available: the project gets MySQL. Dumps usually import; check the caveats below |
| PostgreSQL | **not supported**. Stop here (see below) |
| Redis / Valkey | Redis, optional per project |
| Apache + `.htaccess` | identical: Apache 2.4 with `AllowOverride All` and mod_rewrite |
| nginx-only rules | translate to `.htaccess` or to the project's Apache directives (`lampo project set --apache-directives "<rules>"`) |
| Mailpit, MailHog, Herd/Laragon mail catchers | Mailpit, shared and per project (`--service mailpit`; new Laravel/Symfony/WordPress projects have it): SMTP `127.0.0.1:1025`, web UI `http://127.0.0.1:8025`. MailHog's SMTP side is a drop-in; its UI/API differ |
| Meilisearch, Elasticsearch, OpenSearch, Typesense, MinIO, RabbitMQ, Memcached | not provided: keep running them where they are (Docker, native) and point `.env` at them |
| HTTPS (`herd secure`, Laragon SSL) | not available yet: use `http://` |
| Cron, Supervisor, Horizon | no services: run `php artisan schedule:work` / `queue:work` in a terminal (`lampo:laravel`) |

**PostgreSQL**: say clearly that Lampo does not support PostgreSQL yet, and do not improvise
(no installing PostgreSQL, no converting the schema on your own). Offer the options and let the
user decide: keep the project on its current environment; keep PostgreSQL running where it is
and serve only the PHP side with Lampo (enable `pdo_pgsql` with
`lampo project set --ext pdo_pgsql,pgsql`; Lampo will not manage that database); or move the application to MySQL as a separate,
deliberate project change.

**MariaDB to MySQL**: dumps from MariaDB 10.x/11.x mostly import into MySQL 8.4, but watch for
`uuid`/`inet6` column types, `CHECK` constraints on JSON columns written as
`json_valid(...)`, `SEQUENCE` objects, system-versioned tables, the collation
`utf8mb4_uca1400_ai_ci` (MariaDB 11; replace with `utf8mb4_0900_ai_ci` or `utf8mb4_unicode_ci`
in the dump, with the user's approval) and `DEFINER` clauses. If the import fails (exit 12),
show the error line; fixing the dump is a change the user approves.

Every entry of `warnings` from `lampo project detect` must appear in this check. If something
essential is unsupported, stop and report before creating anything.

## Step 3: register the project

Register the existing folder in place (Laragon, XAMPP, WAMP, Herd: the folder is on Windows
already) or, for a project inside WSL (Sail), clone or copy it to a Windows folder first (see
docker.md). Use the PHP the project ran on, not the default newest:

```powershell
lampo package install php 8.2
lampo project create C:\laragon\www\shop --php 8.2 --activate
```

Add `--preset legacy` for old PHP/MySQL 5.7 projects, `--no-database` if the database will not
move. The JSON result gives the URL and `databaseSetup` (the new, empty database `shop`).

Then reproduce what the inventory found and the preset lacks, in one call (add-ons need
`lampo package install xdebug --php 8.2` first; nginx rules translated to Apache go in
`--apache-directives`):

```powershell
lampo project set shop --ext intl,gd,soap --ini memory_limit=512M --ini upload_max_filesize=64M --service redis --service mailpit
lampo project set shop --public-dir public --service mysql@5.7
```

## Step 4: move the data (confirm first)

1. Export from the source with its own tools, into a backup folder **outside** the project, for
   example `$HOME\lampo-migration\shop`. The reference gives the exact command. Exporting is
   read-only for the source.
2. Check the dump: size, the first lines, `CREATE DATABASE`/`USE` lines (see `lampo:database`).
   A `.sql.gz` or a `.zip` with a single `.sql` (common for hosting backups) imports as is: do
   not extract it.
3. Ask the user, naming the file and the target database, then import:

```powershell
lampo db import $HOME\lampo-migration\shop\shop.sql shop
```

4. Compare a few row counts between source and Lampo (`mysql -e "SELECT COUNT(*) FROM users" shop`).

Uploaded files (`storage\app`, `wp-content\uploads`) stay in the project folder; with a copied
project (Sail) copy them too.

## Step 5: update `.env`

Copy the current `.env` to the backup folder, then change only the connection lines (database,
Redis, mail host and port, `APP_URL`) to the values of `lampo agent context --json`. Typical changes:
`DB_HOST=mysql` / `DB_HOST=db` -> `127.0.0.1`, `DB_USERNAME=sail` -> `root`, empty
`DB_PASSWORD`, `REDIS_HOST=redis` -> `127.0.0.1`, `MAIL_HOST=mailpit` / `mailhog` -> `127.0.0.1`
with `MAIL_PORT` = `mail.smtpPort` (1025) and no `MAIL_USERNAME` / `MAIL_PASSWORD` /
`MAIL_ENCRYPTION`, `APP_URL=http://shop.test` -> the Lampo URL.
Then `php artisan config:clear` (Laravel). WordPress: `wp-config.php` instead (and the site URL
stored in the database, see xampp-wamp.md).

Note that a project registered in place now talks to Lampo's database: the old environment
would need the backed-up `.env` to work again.

## Step 6: ports and the old services

Lampo's Apache needs port 80 and MySQL 8.4 port 3306 (by default); the old stack usually holds
the same ports (a Mailpit/MailHog container or Herd's mail service often holds 1025/8025, which
Lampo's Mailpit needs). Check with `lampo doctor` (`ports`) and `lampo status`. Then, with the user's
yes, stop the old services from their own tool (the reference names it): stopping is
reversible, deleting is not done. Alternatively, if the user prefers to keep the old stack
running, move Lampo's services instead, again only after the user's yes (it changes the ports
of every Lampo project): `lampo settings` shows the current ports, and for example
`lampo settings set --apache-port 8080 --mysql-port 8.4=3310` moves them (checked for
conflicts first; running services restart on the new ports). Use the new MySQL port in the
project's `.env` (`DB_PORT`). Then:

```powershell
lampo up
lampo doctor
```

Also check that the old PHP is not first on PATH (`where.exe php`, doctor `path-shadowing`):
report it; do not edit PATH yourself.

## Step 7: verify

```powershell
lampo agent context --json
php -v
php artisan migrate:status
Invoke-WebRequest -UseBasicParsing http://127.0.0.1:8100 | Select-Object StatusCode
lampo logs php --tail 30
```

Use the project's own URL from `agent context`. Open the main pages, log in if the user gives
credentials, run the test suite against a test database (`lampo:laravel`).

## Step 8: summary and rollback

End with a summary for the user:

- what was created in Lampo: project id, URL, PHP, database name and what was imported;
- files changed in the project (`.env`, `wp-config.php`) and where the backups are;
- services of the old environment that were stopped, and anything left unsupported
  (PostgreSQL, search engines) and how it is handled now; where caught mail is now (the Mailpit
  web UI link);
- how to roll back:
  1. restore `.env` (or `wp-config.php`) from the backup folder;
  2. `lampo project deactivate <id>`, or, only if the user wants it gone from Lampo,
     `lampo project remove <id> --yes` (ask first; the folder and the database stay);
  3. drop the imported database only if the user asks: `lampo db drop <name> --yes`;
  4. start the old environment again from its own tool.
