---
name: laravel
description: Running a Laravel application on Lampo (Windows): .env for Lampo's MySQL and Redis, APP_URL, key generation, migrations and seeders, queue workers and the scheduler in a terminal (no Windows services), tests with a dedicated database, Vite/npm, Xdebug, mail caught by Mailpit, and common Laravel errors on Lampo. Use when the project has an `artisan` file and the user asks to "set up the .env", "run the migrations", "start the queue worker", "run the scheduler", "run the tests", "configure Redis/cache/sessions", "debug with Xdebug", "set up mail", "where do the emails go", or reports a Laravel error (SQLSTATE, No application encryption key, Vite manifest not found, 419, Class Redis not found).
---

# Laravel on Lampo

Core rules: `lampo:lampo`. Get the facts first:

```powershell
lampo agent context --json
```

Use `url`, `database`, `mail` and `services` from it. `php` and `composer` are shims with the
project's PHP: run `php artisan ...` directly in the project folder (or in a worktree with its
own environment, `lampo:worktrees`).

## First setup of a checkout

```powershell
composer install
Copy-Item .env.example .env
php artisan key:generate
php artisan migrate
```

Copy `.env.example` only when `.env` does not exist, and run `key:generate` only when `APP_KEY`
is empty: regenerating an existing key breaks encrypted data and sessions (ask first).

## `.env` for Lampo

Write these from `agent context` (allowed; keep every other line):

```dotenv
APP_URL=http://shop.localhost

DB_CONNECTION=mysql
DB_HOST=127.0.0.1
DB_PORT=3306
DB_DATABASE=shop
DB_USERNAME=lampo_shop
DB_PASSWORD=<database.password>
```

`DB_USERNAME` and `DB_PASSWORD` are the `database.user` and `database.password` of `agent
context --json` (a new project has its own account `lampo_<id>` with a random password; a
project still on root has `root`, with an empty or random password): copy them, never guess.

- `APP_URL` = the `url` of the site (in a worktree environment, its `http://127.0.0.1:<port>`);
  it matters for generated links, `asset()`, mail links, Sanctum/Fortify (`SANCTUM_STATEFUL_DOMAINS`
  if the app sets it) and the Vite dev server.
- If `DB_CONNECTION=sqlite` (the default of new Laravel apps), switch to `mysql` with the values
  above when the user wants MySQL; otherwise leave SQLite alone (it works with Lampo's PHP).
- Redis, only if the project uses it (`services` lists `redis`, default port 6379):

```dotenv
REDIS_CLIENT=predis
REDIS_HOST=127.0.0.1
REDIS_PORT=6379
REDIS_PASSWORD=null
```

  `REDIS_CLIENT=phpredis` (Laravel's default) needs the `php-redis` add-on enabled for the
  project (`lampo:php`); `predis` needs `composer require predis/predis`, which changes the
  project's dependencies: ask before adding it. If the project has no Redis service and the
  task needs it, enable it with `lampo project set --service redis` (say so); otherwise use
  `CACHE_STORE=database` / `SESSION_DRIVER=database` / `QUEUE_CONNECTION=database` (or `file`).
- After editing `.env`: `php artisan config:clear` (a cached config ignores `.env`).

## Migrations and seeders

```powershell
php artisan migrate
php artisan db:seed
php artisan migrate:status
```

Running pending migrations is normal work. `migrate:fresh`, `migrate:refresh`,
`migrate:reset`, `db:wipe` and `--seed` on top of existing data destroy data: ASK first, unless
the database is a throwaway one (a worktree `fresh`/`clone:` database or a test database).
Export first when in doubt: `lampo db export before-migrate.sql`.

## Queues and the scheduler (no Windows services)

Lampo does not run queue workers or the scheduler as services. Run them in their own terminal,
from the project folder, and leave them running:

```powershell
php artisan queue:work --tries=3
php artisan schedule:work
```

- `queue:work` keeps the code in memory: restart it after code changes, or use
  `php artisan queue:listen` during development (slower, reloads every job).
- `schedule:work` runs `schedule:run` every minute in the foreground; there is no cron on
  Windows to set up.
- As an agent, start them as background processes only if your environment supports that and
  stop them when done; otherwise tell the user which command to run in a terminal. For a quick
  check, process one job with `php artisan queue:work --once` or run `php artisan schedule:run`.
- `QUEUE_CONNECTION=sync` runs jobs inline, handy for debugging.
- Horizon needs `pcntl`, which PHP on Windows does not have: use `queue:work` locally.

## Tests with a dedicated database

Never let `RefreshDatabase` or `migrate:fresh` run against the development database. The
project's account (`lampo_<id>`) reaches only its own database and its worktrees' databases:
`lampo db create shop_testing` makes a database it cannot use (`[1044] Access denied`); any
second database the app connects to has the same limit. Options, in order:

- SQLite in memory (`DB_CONNECTION=sqlite`, `DB_DATABASE=:memory:` in `phpunit.xml`): keep it if
  already configured, use it when the tests allow.
- A worktree environment with its own database (`lampo:worktrees`), when that fits.
- A separate MySQL test database: needs the project on root. Ask the user first, then
  `lampo project set shop --mysql-credentials root` and `lampo db create shop_testing`; copy
  `database.user` / `database.password` from `lampo agent context --json` into `.env` and into
  `.env.testing` or `phpunit.xml` (`DB_DATABASE=shop_testing`, `DB_CONNECTION` `mysql`).

Then:

```powershell
php artisan test
php artisan test --filter=LoginTest
lampo exec -- vendor\bin\pest --parallel
```

Parallel testing creates `shop_testing_test_1`, `_2`... databases: only root can, so it fails
with the project's account (use SQLite in memory, or move the project to root as above).

## Front-end (Vite, npm)

Node.js and npm are not managed by Lampo and are not affected by it: `npm install`,
`npm run dev`, `npm run build` work as usual. `npm run dev` serves assets from
`http://localhost:5173` while the page is served by Apache at `APP_URL`; "Vite manifest not
found" means neither the dev server is running nor `npm run build` has been run.

## Xdebug

Enable the `xdebug` add-on for the project (`lampo:php`), then trigger a session:
`?XDEBUG_SESSION=1` in the browser, or `$env:XDEBUG_SESSION = "1"; php artisan test --filter=X`.
The IDE listens on port 9003.

## Mail

Lampo runs one shared Mailpit: it accepts every mail on plain SMTP and shows it in a web UI;
nothing leaves the machine. When the project uses it, `agent context` has a `mail` object
(`host`, `smtpPort`, `webUrl`) and `services` lists `mailpit`. Read the port from there (the
default is 1025, but the user may have moved it):

```dotenv
MAIL_MAILER=smtp
MAIL_HOST=127.0.0.1
MAIL_PORT=1025
MAIL_USERNAME=null
MAIL_PASSWORD=null
MAIL_ENCRYPTION=null
```

- No authentication and no TLS: leave username, password and encryption empty/`null` (Mailpit
  accepts any credentials anyway). Keep the project's `MAIL_FROM_ADDRESS`.
- The caught mails are at `webUrl` (default `http://127.0.0.1:8025`); give the user that link.
  Mailpit's API there (`/api/v1/messages`) lets you check that a mail was sent.
- Mailpit starts by itself while a served project uses it. If the project does not use it
  (no `mail` in `agent context`), enable it with `lampo project set --service mailpit` (say
  so); `--no-service mailpit` turns it off. Without it use `MAIL_MAILER=log` (mails go to
  `storage\logs\laravel.log`).
- Plain PHP `mail()` needs no setup: Lampo's php.ini for the site already points `SMTP` and
  `smtp_port` at Mailpit.

## Common errors

| Symptom | Fix |
|---------|-----|
| `SQLSTATE[HY000] [2002]` | MySQL not running or wrong port: `lampo status`, `lampo up`; re-read `database.port`. |
| `SQLSTATE[HY000] [1045] Access denied` | `.env` credentials are stale: copy `database.user` and `database.password` from `lampo agent context --json` (`lampo:database`, Troubleshooting), then `php artisan config:clear`. |
| `SQLSTATE[HY000] [1049] Unknown database` | `lampo db create <name>` or fix `DB_DATABASE`. |
| `could not find driver` | `pdo_mysql` not enabled for the project (`lampo:php`). |
| `No application encryption key` | `php artisan key:generate` (only if `APP_KEY` is empty). |
| `Class "Redis" not found` | enable the `php-redis` add-on, or `REDIS_CLIENT=predis` with the package. |
| `419 Page Expired` | `APP_URL` / `SESSION_DOMAIN` do not match the URL in the browser; clear cookies. |
| 404 on every route | document root must be `public` (`lampo:services-logs`). |
| `Connection could not be established with host "mailpit"` / port 1025 refused | `MAIL_HOST=127.0.0.1`, `MAIL_PORT` from `mail.smtpPort`; `lampo up mailpit` if it is not running. |
| Changes to `.env` ignored | `php artisan optimize:clear`. |
| `Vite manifest not found` | `npm run build`, or `npm run dev` running. |
| Composer platform error | the project's PHP is too old or an extension is missing (`lampo:php`). |

Logs: `storage\logs\laravel.log`, then `lampo logs php --tail 50` and `lampo logs error`.
