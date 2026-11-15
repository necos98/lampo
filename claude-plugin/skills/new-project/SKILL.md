---
name: new-project
description: Add a project to Lampo: register an existing folder (`lampo project create`), clone a git repository (`lampo project new --git`) or create a new Laravel application with the Lampo-managed installer (`lampo project new --laravel`), choose preset and PHP version, addressing (.localhost plus a loopback port, or a hosts-file name that needs UAC), the automatic database, and activation. Use when the user asks "add this folder to Lampo", "serve this project", "clone this repo and set it up", "create a new Laravel project", "start a new PHP site", or a lampo command fails with exit code 6 (no project for this folder).
---

# New projects

Core rules: `lampo:lampo`. Registering a project is what the user asked for when they ask to
add, serve, clone or create one; if they did not (for example you only hit exit code 6), ask
before registering a folder. Lampo stores the project in its own config; nothing is written in
the project folder.

## Register an existing folder

```powershell
cd C:\sites\shop
lampo project create --activate
lampo project create C:\sites\blog --preset wordpress --php 8.3 --activate
```

Defaults come from detection: Laravel (`artisan` + `laravel/framework`), WordPress
(`wp-config.php`), Symfony (`bin/console`), any `composer.json`, and Docker files
(`compose.yaml`, `docker-compose.yml`, `Dockerfile`: the PHP of the image, MySQL/Redis
services, warnings for PostgreSQL or Mailpit). Options:

| Option | Effect |
|--------|--------|
| `--name <display name>` | default: the folder name |
| `--id <id>` | used in URLs and the database name (default: from the name, `my-shop`) |
| `--preset <id>` | `laravel`, `wordpress`, `symfony`, `generic-php`, `legacy` (PHP 7.4 + MySQL 5.7), or a custom preset |
| `--php <x.y>` | PHP minor; default: the preset's (the newest), raised to `composer.json` `require.php` |
| `--activate` | serve it right away (otherwise `lampo project activate <id>` later) |
| `--no-database` | do not create the MySQL database |
| `--hosts-name` | also a `<id>.test` hosts-file name: needs `--allow-elevation` (UAC) |

Pass `--php` explicitly when you know the version the project runs on in production or in its
old environment: the default is the newest PHP, which old code may not support. The PHP must
be installed (`lampo package install php 8.2`, exit code 10 otherwise).

The JSON result (`--json`) is the project (id, URLs, PHP, database...) plus `databaseSetup`
(`name`, `status`, `message`), whose `status` is `created`, `already-exists` (kept untouched),
`package-missing` (install the MySQL series, then `lampo db create`) or `failed` (see
`message`; the project itself was created).

Exit code 5 means the folder or id is already registered: `lampo project list`.

## Clone a repository

```powershell
lampo project new --git https://github.com/acme/shop.git --in D:\sites
lampo project new --git git@github.com:acme/shop.git --in D:\sites --branch develop --folder shop-dev --activate
```

git must be on PATH; it runs with no credential prompt, so a private repository needs SSH keys
or a credential manager already set up. Then the folder is registered like `project create`
(`--php`, `--activate`, `--no-database`; `--no-register` only clones). The tool output goes to
stderr; on failure the partially created folder is removed.

After cloning: `composer install`, create `.env` from `.env.example` and fill the database
values (`lampo:database`, `lampo:laravel`), run migrations.

## Create a Laravel application

```powershell
lampo project new --laravel shop --in D:\sites --php 8.4 --activate
lampo project new --laravel shop --in D:\sites --starter-kit react --npm --init-git --activate
```

The Laravel Installer is managed by Lampo (its own tools folder, Lampo's PHP and Composer; the
user's global Composer is never touched). Options: `--starter-kit none|react|vue|livewire|svelte`,
`--phpunit` (instead of Pest), `--init-git` (needs git), `--npm` (needs Node.js; installs and
builds the front-end), `--no-register`. The PHP must be 8.2 or newer. The new app's `.env`
still points at SQLite or `127.0.0.1` defaults: set the database values from
`lampo agent context --json` and run `php artisan migrate` (see `lampo:laravel`).

## Addresses

- Default: `http://<id>.localhost` plus a dedicated loopback port (`http://127.0.0.1:8100`).
  Browsers resolve `*.localhost` to the loopback by themselves: no hosts file, no UAC. Some
  command-line HTTP clients do not; the `127.0.0.1:<port>` URL always works.
- `--hosts-name` adds `<id>.test` (the top-level domain is a Lampo setting) through the hosts
  file. That needs a UAC prompt: ASK the user, and only then add `--allow-elevation`. Without
  it the command exits with 7. If the user refuses the prompt (exit 8), stay on `.localhost`.
- `.test` names may already be used by Laragon or Herd on the same machine: prefer the default
  addressing unless the user wants a hosts name.

## Activate and check

```powershell
lampo project activate shop
lampo project info shop
lampo agent context --json
```

`project deactivate <id>` stops serving it (nothing is deleted). Open the `url` from
`agent context` to verify, and look at `lampo logs php --tail 20` if it fails
(`lampo:services-logs`).

## Not available from the CLI

Removing a project, renaming it, changing its addresses, public folder, extensions, php.ini
values or Apache directives is done in the Lampo app ("Progetti" > project). Removing a project
never deletes its folder or its database.
