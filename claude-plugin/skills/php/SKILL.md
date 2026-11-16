---
name: php
description: PHP toolchain of a Lampo project on Windows: the php/composer/mysql/mysqldump/redis-cli shims, `lampo exec` and `lampo env`, installing PHP versions and add-ons (Xdebug, the Redis extension) with `lampo package install`, enabling extensions and php.ini values (including `mail()` through Mailpit), and switching a project's PHP version. Use when the user asks which PHP runs, "php -v shows the wrong version", "install PHP 8.3", "enable Xdebug", "missing extension intl/gd/redis", "raise memory_limit", "mail() does not work", "switch this project to PHP 8.4", or when composer/php commands must run with the project's toolchain.
---

# PHP on Lampo

Read the core rules in `lampo:lampo` first (autonomy, exit codes). Check the current PHP with
`lampo agent context --json` (`php.version`, `php.binary`, `php.iniDir`, `php.composerPhar`).

## Shims: just call the tools

`php`, `composer`, `mysql`, `mysqldump` and `redis-cli` on PATH are small Lampo executables in
`%LOCALAPPDATA%\Lampo\bin`. On every call they resolve the site from `LAMPO_PROJECT` or the
current folder (see the core skill) and run that site's PHP with its own php.ini (`PHPRC`), its
Composer, and its MySQL/Redis clients already pointed at the right port (`mysql` also selects
the site's database; agent usage in `lampo:database`).

```powershell
php -v
composer install --no-interaction
php artisan migrate
mysql -e "SHOW DATABASES"
```

- Exit code 127 from a shim: it could not resolve a PHP or the tool (no project and no default
  PHP, or the package is missing). Run `lampo agent context --json` and read `hints`.
- Wrong version? `lampo shims status` and `lampo doctor` (check `path-shadowing`): another
  `php.exe` earlier on PATH (XAMPP, Laragon, Herd, a manual install) shadows the shim. Do not
  edit the user's PATH yourself; report what `where.exe php` shows and suggest removing the
  other entry or installing the shims from the Lampo app ("Impostazioni" > "Strumenti da riga
  di comando" > "Installa nel PATH", or `lampo shims install`). A new terminal is needed after
  PATH changes.
- A `LAMPO_PROJECT` variable left in the shell still applies outside every project folder (and
  inside its `LAMPO_PROJECT_ROOT`): check `$env:LAMPO_PROJECT` when a folder outside projects
  gets a project's PHP.

## `lampo exec` and `lampo env`

For anything that is not a shim (a script that calls `php.exe` by path, `vendor\bin\pest`, a
tool that spawns `php` itself), run it with the site's toolchain first on PATH:

```powershell
lampo exec -- vendor\bin\phpunit
lampo exec -- composer run-script post-install-cmd
```

Everything after `--` is the command line; stdio is inherited; the exit code is the child's
(`--json` is ignored). `lampo env` prints the same environment (PATH additions, `PHPRC`,
`LAMPO_PROJECT`, `LAMPO_PROJECT_ROOT`, `LAMPO_PHP_VERSION`, `LAMPO_DB_HOST`, `LAMPO_DB_PORT`, `LAMPO_MAIL_HOST`,
`LAMPO_MAIL_PORT`...). To load it into
the current shell:

```powershell
lampo env --shell powershell | Out-String | Invoke-Expression
```

In Git Bash use `eval "$(lampo env --shell bash)"`. Lampo only sets `PHPRC` and `LAMPO_*`
variables, never framework variables such as `DB_HOST`.

## PHP versions

Any PHP from 5.6 up, per project. List and install (downloads go to the user's data folder, no
admin rights):

```powershell
lampo package list --kind php
lampo package list --installed
lampo package install php 8.3
```

`package list` shows the catalog; `--installed` shows only what is on this machine, with the
sites using each build (`--kind php --installed` for PHP alone). A two-part value (`8.3`)
installs the latest patch of that series; three parts (`8.3.14`) an exact build. Installing is
allowed when the task needs it. Uninstalling needs the user's yes first:

```powershell
lampo package uninstall php 8.2 --yes
lampo package uninstall xdebug --php 8.3 --yes
```

It takes the same arguments as `install`, but exactly one installed build must match (several:
exit 4 listing them, pass the exact version). It is refused with exit 5 while the build is the
last one a project, the default PHP or an always-on MySQL needs (`details.usedBy`).

### Switching a project's PHP

```powershell
lampo project set-php 8.4 shop
```

ASK the user before changing the PHP of a main project: it changes what every request and
every shim call uses. Check first that the code supports it (`composer.json` `require.php`,
`composer check-platform-reqs` after the switch). Worktree environments inherit the parent's
PHP. The version must be installed; exit code 10 means `lampo package install php <x.y>`.

## Extensions and add-ons

- **Bundled extensions** (`intl`, `gd`, `zip`, `pdo_mysql`, `mbstring`, `sodium`, `exif`...):
  the presets enable the usual ones. `lampo package extensions` lists what the project's PHP
  build ships, marking those the project enables (`--php 8.4` for another installed minor,
  `--json` for `extensions[].name` / `enabled`); read-only, no app needed. Enable or disable
  others with `lampo project set` (comma-separated or repeated; a name the build does not ship
  fails with exit 4 and lists the available ones). Verify with `php -m`.

```powershell
lampo package extensions
lampo project set --ext intl,gd
lampo project set --no-ext opcache
```

- **Add-ons** are separate downloads built per PHP minor: `xdebug` and `php-redis` (the phpredis
  extension, needed only when the app uses `REDIS_CLIENT=phpredis`; Laravel can use `predis`
  instead, a Composer package). Install the build for the project's PHP, then enable it (exit
  10 from `--addon` means the build for that PHP is missing: the message names the command):

```powershell
lampo package list --kind xdebug
lampo package install xdebug --php 8.3
lampo project set --addon xdebug
lampo package install php-redis --php 8.3
lampo project set --addon php-redis
lampo project set --no-addon xdebug
```

`lampo project set` changes only what it names, regenerates the site's php.ini and Apache
config (restarting Apache when it runs), and prints the updated project (`--json`: the
project document). It acts on the resolved project, or on the id given first
(`lampo project set <id> --ext intl`); worktree environments use their parent's settings.

- Extensions that windows.php.net builds do not ship (for example `imagick`, `swoole`) are not
  available through Lampo: say so instead of downloading DLLs into the project.

## Xdebug

With the add-on enabled, Lampo writes Xdebug 3 defaults: `xdebug.mode=debug`,
`xdebug.start_with_request=trigger`, `xdebug.client_host=127.0.0.1`, `xdebug.client_port=9003`
(Xdebug 2 equivalents, port 9000, on PHP 5.6-7.1). Debugging starts only on a trigger:

- browser: the Xdebug helper extension, or `?XDEBUG_SESSION=1` on the URL;
- CLI: `$env:XDEBUG_SESSION = "1"; php artisan my:command` (PowerShell), or
  `php -d xdebug.start_with_request=yes script.php`.

The IDE listens on port 9003 (VS Code "PHP Debug": `"port": 9003`, path mappings not needed
since Apache serves the real folder). Apache allows long requests, so a paused breakpoint does
not end in a 500. To profile, add `xdebug.mode=debug,profile` as a php.ini value (below).

## php.ini values

The site's php.ini is generated by Lampo (the folder is `php.iniDir` in `agent context`; never
edit it, it is overwritten). Per-project overrides such as `memory_limit`,
`upload_max_filesize`, `post_max_size`, `max_execution_time`, `xdebug.mode` are set with
`--ini KEY=VALUE` (repeatable; the value is taken verbatim, commas included) and removed with
`--no-ini KEY`. They apply to the site and to its shims. For a single CLI run, `-d` is enough:

```powershell
lampo project set --ini memory_limit=512M --ini upload_max_filesize=64M --ini post_max_size=64M
lampo project set --ini xdebug.mode=debug,profile
lampo project set --no-ini memory_limit
php -d memory_limit=-1 artisan migrate
php --ini
php -i
```

### `mail()`

PHP's `mail()` on Windows speaks SMTP to the `SMTP` / `smtp_port` php.ini values. For a site
that uses Mailpit (`lampo project set --service mailpit`), Lampo's php.ini already sets
`SMTP = 127.0.0.1`, `smtp_port = <Mailpit's SMTP port>` (1025 by default) and
`sendmail_from = lampo@localhost`, so `mail()` works with no setup and the message appears in
Mailpit's web UI (`mail.webUrl` in `agent context`). A project's own `--ini SMTP=...` /
`--ini smtp_port=...` / `--ini sendmail_from=...` values win. Sites without Mailpit get none
of these (PHP's defaults: `localhost:25`, so `mail()` fails). Check with
`php -r "var_dump(mail('test@example.com', 'Test', 'Hello'));"` (prints `bool(true)`).

## Composer

`composer` uses Lampo's Composer for the project's PHP (`php.composerChannel`: stable, or the
2.2 LTS line for old PHP). Run it in the project folder. Platform errors
("your php version does not satisfy") mean the project's PHP is too old or an extension is
missing: compare with `php -v` / `php -m` before changing anything. Never pass
`--ignore-platform-reqs` to hide a real mismatch without telling the user.
