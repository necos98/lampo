# From Docker / docker-compose / Laravel Sail

Part of `lampo:migrate`. Read-only on the containers: never `docker compose down -v`, never
`docker volume rm`, never edit the compose file or Dockerfile.

## Inventory

Files in the project (Lampo's detection reads them too):

- `compose.yaml`, `compose.yml`, `docker-compose.yml`, `docker-compose.yaml`, plus overrides
  (`docker-compose.override.yml`): `services:` with their `image:`, `build:`, `ports:`,
  `environment:`, `volumes:`.
- `Dockerfile`, `docker/**/Dockerfile`, `.docker/**`: the `FROM` line gives the PHP
  (`php:8.2-apache`, `php:8.3-fpm-alpine` -> PHP 8.2 / 8.3); `docker-php-ext-install`,
  `docker-php-ext-enable` and `pecl install` lines list the extensions (`pdo_mysql`, `gd`,
  `intl`, `zip`, `redis`, `xdebug`...); `COPY ... php.ini` / `conf.d/*.ini` give ini values.
- nginx or Apache config copied into an image (`docker/nginx/default.conf`, `*.conf`): rules to
  translate (see `lampo:services-logs`, rewrites).
- `.env`: `DB_HOST` is the compose service name (`mysql`, `db`, `mariadb`), `REDIS_HOST=redis`,
  `MAIL_HOST=mailpit`.

Running state (if Docker is running):

```powershell
docker compose ps
docker compose config --services
docker compose exec app php -v
docker compose exec app php -m
```

Lampo's reading of the same files, nothing registered (`services`, `warnings`, and
`phpVersion` from the PHP image or the Sail runtime):

```powershell
lampo project detect C:\sites\shop --json
```

### A compose file with several PHP apps

When the compose file runs more than one PHP app, each from its own folder, `detect --json`
adds `apps`: one proposed project per app. Without `apps` there is a single app (a queue
worker or scheduler sharing the web service's folder is not a separate app): use the
top-level fields as above.

```json
"apps": [
  {"service": "shop", "folder": "apps/shop", "phpVersion": "8.3", "extensions": ["intl", "pdo_mysql"],
   "documentRoot": "public", "environment": {"DB_HOST": "mysql", "DB_DATABASE": "shop"},
   "serviceHosts": ["mysql", "redis"]}
]
```

- `folder`: the app's code, relative to the analysed folder (from the volume mounted on
  `/var/www/html`, `/app`... or the `build` context); a path written outside the folder is
  kept as written: check it exists on Windows.
- `phpVersion`, `extensions`: from the image, the Sail runtime or the context's Dockerfile
  (`docker-php-ext-install`; `pecl install` and `install-php-extensions` lines are not read:
  check the Dockerfile for those).
- `documentRoot`: relative to `folder` (`.` = the folder itself), from `APACHE_DOCUMENT_ROOT`,
  a mounted Apache `DocumentRoot` or the nginx `root` of the block passing PHP to the service;
  `null` when none was found (ask, or look at the app: `public/`, `web/`).
- `environment`: the service's `environment:` as written; values that name a compose service
  (`DB_HOST=mysql`) become `127.0.0.1` on Lampo (see `.env` changes).
- `serviceHosts`: the compose services the app reaches (databases, Redis, Mailpit): map each
  one with the image table below.

Register each app as its own project (one per folder), confirming the list with the user
first; the plain-text `lampo project detect` prints the command for each app:

```powershell
lampo project create C:\sites\stack\apps\shop --php 8.3 --json
lampo project set shop --public-dir public --ext intl,pdo_mysql --json
```

Each Lampo project gets its own database, even when the apps shared one database container:
export and import each app's database separately (below).

Image to Lampo mapping: `php:<x.y>-*` -> PHP x.y; `mysql:5.7` -> MySQL 5.7 (`--preset legacy`
or the 5.7 service); `mysql:8.x` -> MySQL 8.4; `mariadb:*` -> MySQL 8.4 (caveats in
SKILL.md); `postgres:*` -> **not supported** (stop, SKILL.md); `redis:*` / `valkey/*` -> Redis;
`axllent/mailpit` -> Mailpit (`--service mailpit`); `mailhog/mailhog` -> Mailpit too (same SMTP
usage, port 1025; MailHog's web UI and API on 8025 differ, so scripts calling MailHog's API need
updating);
`meilisearch`, `elasticsearch`, `minio`... -> not provided, keep them in
Docker and point `.env` at their published port (`127.0.0.1:<host port>`).

## Laravel Sail

- Sail's compose file is `compose.yaml` or `docker-compose.yml`; the app service is
  `laravel.test`, built from `vendor/laravel/sail/runtimes/<x.y>` (the PHP version; older Sail
  uses images such as `laravelsail/php82-composer`).
- Sail defaults in `.env`: `DB_HOST=mysql`, `DB_USERNAME=sail`, `DB_PASSWORD=password`,
  `REDIS_HOST=redis`, `MAIL_HOST=mailpit`, `APP_PORT`/`FORWARD_DB_PORT` for published ports.
  On Lampo: `MAIL_HOST=127.0.0.1`, `MAIL_PORT=1025` (`mail.smtpPort` in `agent context`);
  Sail's `mailpit` container and `FORWARD_MAILPIT_PORT` / `FORWARD_MAILPIT_DASHBOARD_PORT` are no
  longer needed (the web UI is `http://127.0.0.1:8025`). Stop the container before `lampo up`
  if it publishes 1025 or 8025.
- On Windows a Sail project normally lives in WSL (`\\wsl$\Ubuntu\home\<user>\shop`, or
  `\\wsl.localhost\...`). Lampo serves Windows folders: clone the repository into a Windows
  folder (`lampo project new --git <url> --in C:\sites`) or copy it (with the user's agreement,
  for uncommitted work), then run `composer install` and `npm ci` again on Windows (`vendor`
  and `node_modules` built in Linux contain Linux binaries).
- `./vendor/bin/sail` commands are not needed afterwards: `php artisan ...` with Lampo's shims.

## Export the database

From the running database container, with the credentials of the compose file / `.env`.
Write the dump inside the container and copy it out: a PowerShell `>` redirection would
re-encode it as UTF-16.

```powershell
New-Item -ItemType Directory -Force $HOME\lampo-migration\shop
docker compose exec -T mysql mysqldump -usail -ppassword --single-transaction --routines --triggers --no-tablespaces --result-file=/tmp/shop.sql laravel
docker compose cp mysql:/tmp/shop.sql $HOME\lampo-migration\shop\shop.sql
```

Replace `mysql` (service), `sail`/`password` (user, password) and `laravel` (database) with
the project's values. MariaDB images ship `mariadb-dump` (older ones also `mysqldump`). If the
database container is not running, `docker compose up -d mysql` starts only that service
(ask: it changes the user's running containers). Do not pass `--databases`, so the dump can go
into any target database.

Then follow SKILL.md step 4 (confirm, `lampo db import`).

## Ports and stopping

Containers often publish `80`, `3306`, `6379`. With the user's yes:

```powershell
docker compose stop
```

`stop` keeps containers and volumes (`docker compose start` brings them back). Never use
`down -v`. Keep the services Lampo does not provide (Meilisearch...) running if the app
needs them: `docker compose stop laravel.test mysql redis mailpit` stops only the replaced ones
(the Mailpit container must stop too: it holds ports 1025/8025).

## `.env` changes (after a backup)

`DB_HOST=127.0.0.1`, `DB_PORT` from Lampo, `DB_USERNAME=root`, `DB_PASSWORD=` (empty),
`DB_DATABASE` = the Lampo database, `REDIS_HOST=127.0.0.1`, `APP_URL` = the Lampo URL,
`MAIL_MAILER=smtp`, `MAIL_HOST=127.0.0.1`, `MAIL_PORT` = `mail.smtpPort` from Lampo (1025), no
username, password or encryption. Remove nothing else; `php artisan config:clear`.

## Rollback

Restore `.env` from `$HOME\lampo-migration\shop`, `docker compose start`, deactivate the Lampo
project. The containers and volumes were never touched.
