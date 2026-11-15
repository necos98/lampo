---
name: php
description: PHP toolchain of a Lampo project on Windows: the php/composer/mysql/mysqldump/redis-cli shims, `lampo exec` and `lampo env`, installing PHP versions and add-ons (Xdebug, the Redis extension) with `lampo package install`, enabling extensions and php.ini values, and switching a project's PHP version. Use when the user asks which PHP runs, "php -v shows the wrong version", "install PHP 8.3", "enable Xdebug", "missing extension intl/gd/redis", "raise memory_limit", "switch this project to PHP 8.4", or when composer/php commands must run with the project's toolchain.
---

# PHP on Lampo

Read the core rules in `lampo:lampo` first (autonomy, exit codes). Check the current PHP with
`lampo agent context --json` (`php.version`, `php.binary`, `php.iniDir`, `php.composerPhar`).

## Shims: just call the tools

`php`, `composer`, `mysql`, `mysqldump` and `redis-cli` on PATH are small Lampo executables in
`%LOCALAPPDATA%\Lampo\bin`. On every call they resolve the site from `LAMPO_PROJECT` or the
current folder (see the core skill) and run that site's PHP with its own php.ini (`PHPRC`), its
Composer, and its MySQL/Redis clients already pointed at the right port.

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
- A stale `LAMPO_PROJECT` variable in the shell overrides the folder: check `$env:LAMPO_PROJECT`.

## `lampo exec` and `lampo env`

For anything that is not a shim (a script that calls `php.exe` by path, `vendor\bin\pest`, a
tool that spawns `php` itself), run it with the site's toolchain first on PATH:

```powershell
lampo exec -- vendor\bin\phpunit
lampo exec -- composer run-script post-install-cmd
```

Everything after `--` is the command line; stdio is inherited; the exit code is the child's
(`--json` is ignored). `lampo env` prints the same environment (PATH additions, `PHPRC`,
`LAMPO_PROJECT`, `LAMPO_PHP_VERSION`, `LAMPO_DB_HOST`, `LAMPO_DB_PORT`...). To load it into
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
lampo package install php 8.3
```

A two-part value (`8.3`) installs the latest patch of that series; three parts (`8.3.14`) an
exact build. Installing is allowed when the task needs it; uninstalling is not (ask the user,
it happens in the app's "Pacchetti" page).

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
  the presets enable the usual ones. To enable another one for a project, use the Lampo app:
  "Progetti" > project > "PHP" > "Estensioni" (there is no CLI command yet). Verify with
  `php -m`.
- **Add-ons** are separate downloads built per PHP minor: `xdebug` and `php-redis` (the phpredis
  extension, needed only when the app uses `REDIS_CLIENT=phpredis`; Laravel can use `predis`
  instead, a Composer package). Install the build for the project's PHP, then enable it in
  "Progetti" > project > "PHP" > "Componenti aggiuntivi":

```powershell
lampo package list --kind xdebug
lampo package install xdebug --php 8.3
lampo package install php-redis --php 8.3
```

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
`upload_max_filesize`, `post_max_size`, `max_execution_time`, `xdebug.mode` are set in
"Progetti" > project > "PHP" > "Valori php.ini" (no CLI command yet). For a single CLI run,
`-d` is enough:

```powershell
php -d memory_limit=-1 artisan migrate
php --ini
php -i
```

## Composer

`composer` uses Lampo's Composer for the project's PHP (`php.composerChannel`: stable, or the
2.2 LTS line for old PHP). Run it in the project folder. Platform errors
("your php version does not satisfy") mean the project's PHP is too old or an extension is
missing: compare with `php -v` / `php -m` before changing anything. Never pass
`--ignore-platform-reqs` to hide a real mismatch without telling the user.
