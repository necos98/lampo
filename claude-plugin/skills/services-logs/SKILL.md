---
name: services-logs
description: Lampo services, logs and troubleshooting: `lampo status`, `lampo up/down/restart`, `lampo logs` (Apache, per-site access/error/PHP logs, MySQL, Redis, Mailpit), the Mailpit web UI for caught mail, `lampo doctor`, and a playbook for HTTP 500 errors, 404s and broken rewrites, ports already in use, MySQL that will not start, the wrong PHP version and missing extensions. Use when the user says "the page gives a 500 error", "blank page", "404 on every route", "the site does not open", "port 80/3306 is in use", "MySQL does not start", "show me the logs", "restart Apache", "where are the emails", "open Mailpit", or anything else is broken in the local Lampo environment.
---

# Services, logs and troubleshooting

Core rules: `lampo:lampo`. Start every investigation with:

```powershell
lampo agent context --json
lampo status
```

`hints` in `agent context` often names the problem (project not active, no worktree
environment, Composer missing). `status` lists each service (`apache`, `mysql-8.4`, `redis`, `mailpit`...)
with state, port and PID, and the projects; without the app running the states are `unknown`.

## Services

```powershell
lampo up
lampo up apache mysql-8.4
lampo restart apache
lampo down redis
```

`lampo up` without arguments starts every service the active projects need (it starts the app
headless if needed). Starting and restarting are allowed. Stopping (`lampo down`, `lampo quit`)
affects every project: only when the user asks or the task needs it. Apache is restarted
automatically when a project's configuration changes; `lampo restart apache` is rarely needed.

## Mail (Mailpit)

One shared Mailpit catches the mail of every project that uses it (`--service mailpit`; new
Laravel, Symfony, WordPress and generic PHP projects have it): SMTP on 127.0.0.1:1025, web UI
at http://127.0.0.1:8025 by default. `agent context` gives the actual values in `mail`
(`smtpPort`, `webUrl`); `lampo status` shows `1025, ui 8025` in the port column. Messages are
kept across restarts (a database in Lampo's data folder, newest 500).

```powershell
lampo up mailpit
lampo logs mailpit --tail 30
Invoke-RestMethod http://127.0.0.1:8025/api/v1/messages
```

It starts by itself with the projects that use it. "Where did my email go": open the web UI
link for the user, or query the API above (`messages[].Subject`, `To`) to check a mail arrived.
Nothing is ever delivered to real recipients. PHP `mail()` already uses it through php.ini;
frameworks need `MAIL_MAILER=smtp`, `MAIL_HOST=127.0.0.1`, `MAIL_PORT=<smtpPort>`, no
authentication or encryption (`lampo:laravel`).

## Logs

```powershell
lampo logs
lampo logs php --tail 50
lampo logs error --since 10m
lampo logs access -n 20
lampo logs apache-error -n 50
lampo logs mysql-8.4-error -n 80
lampo logs php:shop@fix-login --tail 30
lampo logs php --tail 200 --level errors
```

`lampo logs` alone lists the sources and files of the resolved site. Site sources: `access`,
`error` (Apache's error log for the virtual host, including PHP-CGI crashes and rewrite
errors), `php` (PHP's own error log: warnings, fatals, uncaught exceptions); add `:<site>` for
another site. Service sources: `apache`, `apache-error`, `mysql-<series>`,
`mysql-<series>-error`, `redis`, `mailpit`. `--follow` (`-f`) streams until Ctrl+C: avoid it in agent runs,
use `--tail` / `--since` instead. `--level errors` (or `warnings`: warnings and errors) keeps
only those among the `--tail` lines, with the rules of the app's log viewer. `--open` and
`--open-folder` open the log file or its folder in the user's apps: only when the user asks. Framework logs live in the project (Laravel:
`storage\logs\laravel.log`): read them directly.

## Doctor

```powershell
lampo doctor
lampo doctor --json
```

Checks Visual C++ runtimes, ports, shims on PATH and PATH shadowing, the hosts file, required
packages, stale worktrees and project folders. Exit code 14 means at least one `[error]`; each
problem has a `fix:` line. Act on fixes within your autonomy (installing a missing package,
starting services); report the others (VC++ downloads, other programs on a port).

## Playbook

### HTTP 500 or a blank page

1. `lampo logs php --tail 50` and `lampo logs error --tail 50`.
2. The framework log (Laravel `storage\logs\laravel.log`).
3. Common causes: missing `vendor` (`composer install`), missing `.env` or `APP_KEY` (Laravel:
   copy `.env.example`, then `php artisan key:generate` only when `APP_KEY` is empty: never
   regenerate an existing key without asking), wrong database settings (`lampo:database`), a
   PHP version or extension the code needs (`lampo:php`), a stale config cache
   (`php artisan optimize:clear`).
4. Fix, then reload the page (Apache does not need a restart for code changes).

### 404 on every route but the home page, or rewrites not working

- The document root must be the folder with the front controller: compare `documentRoot` in
  `agent context` with the framework (Laravel and Symfony: `public`). Change it with
  `lampo project set --public-dir public` (ask first on a main project; `""` = the folder
  itself).
- Lampo serves with `AllowOverride All` and mod_rewrite: the project's `.htaccess` works. Check
  it exists in the document root (Laravel ships `public\.htaccess`; WordPress writes it from
  "Settings > Permalinks").
- Rules written for nginx (`try_files`, `location` blocks) do nothing under Apache: translate
  them to `.htaccess` rules in the project or, with the user's yes, to the project's Apache
  directives: `lampo project set --apache-directives "<text>"` replaces them all (`""` clears;
  from a file in PowerShell: `--apache-directives (Get-Content .\lampo.conf -Raw)`). Lampo
  validates them with `httpd -t`; a rejected configuration comes back as exit 12 with
  `details.output` (the project is saved: fix or clear the directives). Directives that run
  programs, load code or expose folders are refused up front (validation error, nothing
  saved): `Include`, `LoadModule`, `Define`, piped or out-of-folder logs (`CustomLog "|…"`),
  `ScriptAlias`, `RewriteMap`, `Fcgid*`, `<Directory>`, `Alias` or `RewriteRule` to a disk
  path, the `file` expression function, `SetEnv PATH`/`PHPRC`, `ServerName`/`ServerAlias`
  (change the project's addresses instead), an `<If>` nested in another `<If>`/`<Else>`
  (combine the conditions with `&&`). The error names the line and the directive
  (`details.reason`). Use `<Location>` blocks or `.htaccess` instead. Whatever they grant,
  `.env*`, `.git*`, `.ht*`, logs and `.sql`/`.bak` files stay denied (403).

### The site does not open at all

- Project not active: `hints` says so; `lampo project activate <id>`.
- Apache not running: `lampo status`, then `lampo up`.
- A `<name>.test` hosts-file address not resolving: use the `.localhost` URL or the
  `127.0.0.1:<port>` URL from `agent context` (they never need the hosts file).
- Use `http://`, not `https://`: local HTTPS is not available yet.

### Port already in use (exit 5, doctor `ports` error, a service that stops at once)

Find the owner of the port (80 Apache, 3306 MySQL 8.4, 6379 Redis, 1025/8025 Mailpit, or the one
reported):

```powershell
Get-NetTCPConnection -LocalPort 80 -State Listen | Select-Object LocalAddress, OwningProcess
Get-Process -Id (Get-NetTCPConnection -LocalPort 80 -State Listen).OwningProcess
```

Typical owners: Laragon, XAMPP or WAMP (their Apache/MySQL), IIS (`W3SVC`), Docker Desktop
containers publishing the port, a Windows MySQL service (`MySQL80`), Herd's nginx. Tell the
user what you found and ASK before stopping anything that is not Lampo; it is their software.
Alternatives: stop the other program from its own UI, or, with the user's yes, move Lampo's
service to a free port: `lampo settings` shows the ports, then for example
`lampo settings set --apache-port 8080`, `lampo settings set --mysql-port 8.4=3310` or
`lampo settings set --redis-port 6380`; Mailpit has two ports, SMTP and its web UI:
`lampo settings set --port mailpit=2025 --ui-port mailpit=9025` (a port taken by another program or Lampo site is
refused, exit 5). Then `lampo up`, and update `DB_PORT` / `REDIS_PORT` / `MAIL_PORT` / `APP_URL` in the
`.env` files of the sites the output lists.

### MySQL does not start

1. `lampo logs mysql-8.4-error -n 80` (use the series of the project).
2. Port taken: see above (another MySQL or MariaDB is the usual cause).
3. "VCRUNTIME"/DLL errors: `lampo doctor` (`vc-runtime`), the user installs the Visual C++
   redistributable from the link given.
4. Never delete or edit files in Lampo's MySQL data folder; ask the user if the log shows
   corruption.

### Wrong PHP version

- `php -v` differs from `php.version` in `agent context`: another `php.exe` is earlier on PATH
  (`where.exe php`, doctor `path-shadowing`) or `LAMPO_PROJECT` is set in the shell (it applies
  outside every project folder). See
  `lampo:php`.
- The project itself uses the wrong minor: `lampo project set-php <x.y> <id>` after asking.

### Missing extension ("Class Redis not found", "could not find driver", "intl required")

`php -m` lists what is loaded. Enable the bundled extension (`pdo_mysql`, `intl`, `gd`,
`zip`...) or the add-on (`xdebug`, `php-redis`) for the project: see `lampo:php`.

### App not running or version mismatch

Exit 9: ask the user to open the Lampo app (or check `--no-start`). Exit 15: the CLI and the
app are different versions; the user updates Lampo and restarts the app.
