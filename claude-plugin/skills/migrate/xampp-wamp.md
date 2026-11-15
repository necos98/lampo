# From XAMPP or WampServer

Part of `lampo:migrate`. Paths are the usual defaults (`C:\xampp`, `C:\wamp64`); check they
exist. Read only: never edit their config files or data folders.

## XAMPP

| What | Where (typically) |
|------|-------------------|
| Projects | `C:\xampp\htdocs\<project>` (served as `http://localhost/<project>/`) |
| Virtual hosts | `C:\xampp\apache\conf\extra\httpd-vhosts.conf`; main config `C:\xampp\apache\conf\httpd.conf` |
| PHP (one version) | `C:\xampp\php\php.exe`, `C:\xampp\php\php.ini` |
| Database | **MariaDB** in current XAMPP releases, under `C:\xampp\mysql\` (`bin\mysqldump.exe`, `bin\mysql.exe`, config `bin\my.ini`, data `data\`) |
| Control panel | `C:\xampp\xampp-control.exe`; Apache/MySQL may also be installed as Windows services |
| Mail | `C:\xampp\sendmail\` (not provided by Lampo) |

```powershell
C:\xampp\php\php.exe -v
C:\xampp\php\php.exe -m
C:\xampp\mysql\bin\mysql.exe -u root -e "SELECT VERSION(); SHOW DATABASES"
Select-String -Path C:\xampp\apache\conf\extra\httpd-vhosts.conf -Pattern 'DocumentRoot|ServerName'
```

Defaults: user `root`, empty password, port 3306 (check `port=` in `my.ini`).

## WampServer

| What | Where (typically) |
|------|-------------------|
| Projects | `C:\wamp64\www\<project>` |
| Virtual hosts | `C:\wamp64\bin\apache\apache2.4.<x>\conf\extra\httpd-vhosts.conf` |
| PHP versions | `C:\wamp64\bin\php\php8.<x>.<y>\` (the one in use is chosen in the tray menu; its `phpForApache.ini` is the web php.ini) |
| MySQL / MariaDB | `C:\wamp64\bin\mysql\mysql<version>\bin\` and `C:\wamp64\bin\mariadb\mariadb<version>\bin\`; both can run at once on different ports (read `port` in each `my.ini`; often MariaDB 3306 and MySQL 3308, or the reverse) |
| Services | Windows services such as `wampapache64`, `wampmysqld64`, `wampmariadb64` |

Find which engine and port the project uses from its `.env` / `wp-config.php` (`DB_PORT`,
`DB_HOST=localhost:3308`).

## Detect, then register in place

XAMPP and WAMP leave no marker in the project folder. See what Lampo detects (framework,
`composer.json` PHP minimum; a WordPress site usually has no `composer.json`, so no PHP), then
pass the PHP the old stack ran and, if `presetId` is wrong or `null`, the preset:

```powershell
lampo project detect C:\xampp\htdocs\shop --json
lampo project detect C:\wamp64\www\blog --json
lampo project create C:\xampp\htdocs\shop --php 8.2 --activate
lampo project create C:\wamp64\www\blog --preset wordpress --php 7.4 --activate
```

A project served at `http://localhost/shop/` (a subfolder) gets its own root URL in Lampo
(`http://shop.localhost`): relative links and `RewriteBase /shop/` in `.htaccess` may need
adjusting; check with the user before editing `.htaccess`.

## Export the database

With the old MySQL/MariaDB running (XAMPP Control Panel "Start" on MySQL, WAMP tray icon green):

```powershell
New-Item -ItemType Directory -Force $HOME\lampo-migration\shop
C:\xampp\mysql\bin\mysqldump.exe -h 127.0.0.1 -P 3306 -u root --single-transaction --routines --triggers --result-file="$HOME\lampo-migration\shop\shop.sql" shop
```

For WAMP use the `mysqldump.exe` (or `mariadb-dump.exe`) of the engine and port the project
uses. XAMPP and many WAMP setups run **MariaDB**: read the MariaDB caveats in SKILL.md before
importing. Then follow SKILL.md step 4.

## WordPress specifics

`wp-config.php` holds `DB_NAME`, `DB_USER`, `DB_PASSWORD`, `DB_HOST`: back it up, then set
`DB_HOST` to `127.0.0.1:3306` (Lampo's port), `DB_USER` `root`, empty `DB_PASSWORD`, `DB_NAME`
the Lampo database. The site URL lives in the database (`wp_options`: `siteurl`, `home`): after
the import, and with the user's approval, change it to the Lampo URL, for example with WP-CLI if
the project has it (`php wp-cli.phar search-replace "http://localhost/blog" "http://blog.localhost" --all-tables`)
or with a targeted `UPDATE` on `wp_options`. Serialized data makes a plain text replace in the
dump unsafe.

## Ports and stopping

XAMPP and WAMP hold 80 and 3306 (WAMP also 3307/3308). With the user's yes they stop them:
XAMPP Control Panel "Stop" (and untick "Svc" if installed as services, which needs admin), WAMP
tray menu "Stop All Services". Their Windows services need administrator rights to stop: do not
try to stop them yourself.

```powershell
Get-Service | Where-Object { $_.Name -match 'wamp|apache|mysql|mariadb' } | Select-Object Name, Status, StartType
```

## PATH

`C:\xampp\php` or a WAMP PHP folder on PATH shadows Lampo's shims (`where.exe php`,
`lampo doctor`). Report it to the user; do not edit PATH.

## Rollback

Restore `.env` / `wp-config.php` from the backup folder, start XAMPP/WAMP again, deactivate the
Lampo project. If the WordPress URLs were changed in the Lampo database, the old database still
has the old ones (it was never modified).
