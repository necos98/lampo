# From Laragon

Part of `lampo:migrate`. Paths below are Laragon's usual defaults (install folder `C:\laragon`);
the user may have installed it elsewhere: check that the folders exist before relying on them.
Read only: do not edit anything under `C:\laragon`.

## Inventory

| What | Where (typically) |
|------|-------------------|
| Projects | `C:\laragon\www\<project>` (the "Document Root" in Laragon's settings may differ) |
| Hostnames | automatic `<project>.test` ("Auto virtual hosts", hostname template in the settings); entries marked `# laragon magic!` in `C:\Windows\System32\drivers\etc\hosts` |
| Virtual hosts | `C:\laragon\etc\apache2\sites-enabled\auto.<project>.test.conf` (Apache) or `C:\laragon\etc\nginx\sites-enabled\` (when Laragon runs nginx) |
| PHP versions | `C:\laragon\bin\php\php-8.x.y-...\` (each with its own `php.ini`) |
| Active versions | `C:\laragon\usr\laragon.ini` (sections such as `[php]`, `[mysql]`, `Version=`) |
| MySQL / MariaDB | `C:\laragon\bin\mysql\mysql-<version>-winx64\bin\` (`mysqldump.exe`, `mysql.exe`); MariaDB builds are also possible, `mariadb-<version>` |
| Database data | `C:\laragon\data\mysql-<major>` or `C:\laragon\data\mysql` (do not touch) |
| Redis, Memcached, mail | `C:\laragon\bin\redis\`, `C:\laragon\bin\memcached\`, Laragon's mail catcher/sendmail settings (on Lampo: Mailpit, `--service mailpit`; PHP `mail()` needs no sendmail) |

Commands (read-only):

```powershell
Get-Content C:\laragon\usr\laragon.ini
Get-ChildItem C:\laragon\bin\php, C:\laragon\bin\mysql
Get-Content C:\laragon\etc\apache2\sites-enabled\auto.shop.test.conf
Select-String -Path C:\Windows\System32\drivers\etc\hosts -Pattern 'laragon'
```

From the vhost: `DocumentRoot` (Laravel: `...\www\shop\public`) and any custom directives.
From the project: `.env` (`DB_DATABASE`, `DB_USERNAME` usually `root`, empty password,
`DB_PORT=3306`, `APP_URL=http://shop.test`).

The PHP version is the one active in Laragon (the menu "PHP" > version) unless the project had
its own; confirm with the user when several are installed. Extensions enabled in Laragon's
`php.ini` (`extension=` lines without `;`) are the ones to enable in Lampo.

Laragon leaves no marker in the project folder, so Lampo's detection sees only the framework and
`composer.json`'s minimum PHP. Check it against what Laragon actually ran:

```powershell
lampo project detect C:\laragon\www\shop --json
```

## Register in place

Laragon projects are normal Windows folders: register the same folder, with Laragon's PHP
(not the detected minimum) when they differ.

```powershell
lampo project create C:\laragon\www\shop --php 8.2 --activate
```

Laragon's `shop.test` keeps pointing at Laragon; Lampo serves `http://shop.localhost` and its
own port. Do not reuse the `.test` name in Lampo (`--hosts-name`) while Laragon's hosts entries
exist: they would clash.

Junctions: Laragon's junctions with an absolute path keep pointing at the old folder after the
project moves (also the Linux symlinks that reached Windows as junctions). List them and recreate
each with its new target:

```powershell
Get-ChildItem C:\sites\shop -Recurse -Attributes ReparsePoint -ErrorAction SilentlyContinue | Select-Object FullName, Target
```

## Export the database

With Laragon's MySQL running (Laragon "Start All", or ask the user to start it), using its own
client:

```powershell
New-Item -ItemType Directory -Force $HOME\lampo-migration\shop
& "C:\laragon\bin\mysql\mysql-8.0.30-winx64\bin\mysqldump.exe" -h 127.0.0.1 -P 3306 -u root --single-transaction --routines --triggers --result-file="$HOME\lampo-migration\shop\shop.sql" shop
```

Adjust the folder to the installed version and the database name. Use `--result-file` (a
PowerShell `>` redirection would produce UTF-16). If Laragon runs MariaDB, read the caveats in
SKILL.md. Do this **before** stopping Laragon and before Lampo's MySQL takes port 3306.

## Ports and stopping

Laragon's Apache/nginx holds port 80 and its MySQL port 3306. With the user's yes, they click
"Stop All" in Laragon (or quit it); you can check what still listens:

```powershell
Get-Process httpd, nginx, mysqld -ErrorAction SilentlyContinue | Select-Object Name, Id, Path
```

Do not kill Laragon's processes yourself unless the user asks; Laragon's "Auto start" settings
are theirs to change. Then `lampo up` and `lampo doctor`.

## PATH

Laragon may have added its `bin\php\...` or `bin\composer` folders to the user PATH ("Add
Laragon to Path"): they shadow Lampo's shims. `where.exe php` and `lampo doctor`
(`path-shadowing`) show it; tell the user (Laragon's menu > Tools > Path > Remove, or edit the
user PATH), do not change it yourself.

## Rollback

Restore `.env` from the backup folder, start Laragon again ("Start All"), and deactivate the
project in Lampo. Laragon's folders, vhosts and databases were never modified.
