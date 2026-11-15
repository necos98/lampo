<#
.SYNOPSIS
  End-to-end scenario of the `lampo` CLI as a coding agent uses it, against the real binaries,
  real downloads and real Apache/PHP/MySQL processes, in an isolated data root.

.DESCRIPTION
  Nothing outside the data root is touched: `LAMPO_HOME` isolates config, packages, databases,
  logs and the control pipe; the user PATH and the registry are never edited (with LAMPO_HOME
  set Lampo leaves the user PATH alone), the hosts file is never written (only `.localhost` and
  loopback-port addressing is used, which needs no elevation) and the shim folder is put on PATH
  only inside this PowerShell process.

  Steps (each one asserts its outcome and the script stops at the first failure):
    1. with the app not running, `lampo package install apache` starts it headless;
    2. installs mod_fcgid, the newest PHP, PHP 7.4, MySQL 8.4 and Composer; installs the shims;
    3. creates two projects from fresh folders on different PHP versions (`.localhost` + port);
    4. fetches a page of each over HTTP (right PHP version, `.htaccess` rewrite working);
    5. `php` and `composer` through the shims resolve per folder;
    6. checks `project create` created the project's database and queries it through the
       `mysql` shim;
    7. creates a git worktree, `lampo worktree up --db clone:<db>`, fetches its own URL and sees
       the worktree's code, shims and the cloned database resolve inside it;
    8. `lampo agent context --json` is right in the project and in the worktree;
    9. `lampo logs` shows the requests;
   10. `lampo down`, then `lampo quit`, leave no httpd/php-cgi/mysqld/redis process behind;
   11. after `lampo up`, killing the app process hard leaves none either.

  Prerequisites: `cargo build --workspace --bins` (debug binaries in target\debug), git and
  curl.exe (both ship with Windows 11 / Git for Windows), network access, ports 80, 3306 and
  8100+ free.

.PARAMETER DataRoot
  The isolated data root (LAMPO_HOME). Default: a new folder under %TEMP%. Packages already in
  it are reused, which makes re-runs fast; everything else is recreated.

.PARAMETER BinDir
  Folder with lampo.exe, lampo-desktop.exe, lampo-shim.exe. Default: <repo>\target\debug.

.PARAMETER Keep
  Leave the app running at the end (otherwise it is stopped and the process checks run).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\e2e-cli.ps1
  powershell -ExecutionPolicy Bypass -File scripts\e2e-cli.ps1 -DataRoot D:\lampo-e2e
#>
[CmdletBinding()]
param(
    [string]$DataRoot = (Join-Path $env:TEMP ("lampo-e2e-" + (Get-Date -Format 'yyyyMMdd-HHmmss'))),
    [string]$BinDir = '',
    [switch]$Keep
)

$ErrorActionPreference = 'Continue'
Set-StrictMode -Version 2.0

$script:step = 0
function Step([string]$title) { $script:step++; Write-Host ""; Write-Host ("[{0,2}] {1}" -f $script:step, $title) -ForegroundColor Cyan }
function Pass([string]$what) { Write-Host "     ok: $what" -ForegroundColor Green }
function Fail([string]$what) {
    Write-Host "     FAILED: $what" -ForegroundColor Red
    Write-Host "     data root: $DataRoot"
    Stop-Stack
    exit 1
}
function Check([bool]$condition, [string]$what) { if ($condition) { Pass $what } else { Fail $what } }

# Runs lampo; returns stdout as one string; fails the scenario on a non-zero exit unless allowed.
function Lampo {
    param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)
    $out = & $lampo @Arguments | Out-String
    if ($LASTEXITCODE -ne 0) { Fail "lampo $($Arguments -join ' ') exited with $LASTEXITCODE`n$out" }
    return $out
}
function LampoJson { param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments) (Lampo @Arguments '--json') | ConvertFrom-Json }

# Runs a native tool (a shim) in a folder and returns its stdout.
function In([string]$dir, [scriptblock]$block) { Push-Location $dir; try { & $block | Out-String } finally { Pop-Location } }

function Http([string]$url) {
    $body = & curl.exe -s --max-time 30 $url | Out-String
    if ($LASTEXITCODE -ne 0) { Fail "GET $url failed (curl exit $LASTEXITCODE)" }
    return $body.Trim()
}

# Processes started from this data root or this build.
function Stack-Processes {
    $roots = @((Join-Path $DataRoot 'data').ToLowerInvariant(), $BinDir.ToLowerInvariant())
    Get-Process -ErrorAction SilentlyContinue | Where-Object {
        $p = $null; try { $p = $_.Path } catch {}
        $p -and ($roots | Where-Object { $p.ToLowerInvariant().StartsWith($_) }) -and $_.ProcessName -ne 'lampo'
    }
}
function Service-Processes { Stack-Processes | Where-Object { $_.ProcessName -in 'httpd', 'php-cgi', 'mysqld', 'redis-server', 'mailpit' } }
function App-Processes { Stack-Processes | Where-Object { $_.ProcessName -eq 'lampo-desktop' } }
function Stop-Stack {
    & $lampo quit 2>$null | Out-Null
    Stack-Processes | Stop-Process -Force -ErrorAction SilentlyContinue
}
function Wait-Until([scriptblock]$condition, [int]$seconds = 20) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) { if (& $condition) { return $true }; Start-Sleep -Milliseconds 250 }
    return [bool](& $condition)
}

# --- setup ------------------------------------------------------------------------------------

if (-not $BinDir) { $BinDir = Join-Path (Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)) 'target\debug' }
$BinDir = (Resolve-Path $BinDir).Path
$lampo = Join-Path $BinDir 'lampo.exe'
foreach ($exe in 'lampo.exe', 'lampo-desktop.exe', 'lampo-shim.exe') {
    if (-not (Test-Path (Join-Path $BinDir $exe))) { Write-Error "$exe not found in ${BinDir}: run cargo build --workspace --bins first"; exit 2 }
}
New-Item -ItemType Directory -Force $DataRoot | Out-Null
$DataRoot = (Resolve-Path $DataRoot).Path
$env:LAMPO_HOME = $DataRoot
$shimDir = Join-Path $DataRoot 'data\bin'
$env:PATH = "$shimDir;$BinDir;$env:PATH"
Remove-Item Env:LAMPO_PROJECT -ErrorAction SilentlyContinue
Write-Host "LAMPO_HOME = $DataRoot"

# A previous run's leftovers: stop its app, start from fresh configuration and sites (packages kept).
Stop-Stack
foreach ($dir in 'config', 'data\mysql', 'data\generated', 'data\logs', 'sites') { Remove-Item -Recurse -Force (Join-Path $DataRoot $dir) -ErrorAction SilentlyContinue }
$sites = Join-Path $DataRoot 'sites'

# --- 1. headless start -------------------------------------------------------------------------

Step 'the app is not running; a mutating command starts it headless'
$status = LampoJson status
Check (-not $status.appRunning) 'app not running at the start'
$apache = LampoJson package install apache
Check ($apache.id.kind -eq 'apache') "apache $($apache.id.version) installed"
Check ($null -ne (App-Processes)) 'lampo-desktop.exe was started by the CLI'
Check ((LampoJson status).appRunning) 'status reports the app as running'
Check (@(App-Processes)[0].MainWindowTitle -ne 'Lampo') 'the app has no Lampo window (headless)'

# --- 2. packages -------------------------------------------------------------------------------

Step 'install mod_fcgid, the newest PHP, PHP 7.4, MySQL 8.4, Composer and the shims'
$null = LampoJson package install mod-fcgid
$phpNew = LampoJson package install php
$newMinor = ($phpNew.id.version -split '\.')[0..1] -join '.'
Pass "newest PHP is $($phpNew.id.version) ($newMinor)"
$php74 = LampoJson package install php 7.4
Check ($php74.id.version -like '7.4.*') "PHP $($php74.id.version)"
$mysql = LampoJson package install mysql 8.4
Check ($mysql.id.version -like '8.4.*') "MySQL $($mysql.id.version)"
$null = LampoJson package install composer
$installed = LampoJson package list --installed
Check (@($installed).Count -ge 6) "$(@($installed).Count) packages installed"
Check (-not (Test-Path (Join-Path $DataRoot 'data\downloads\*.zip'))) 'finished downloads were cleaned up'
$shims = LampoJson shims install
Check ($shims.installed) "shims installed in $($shims.shimDir)"
Check ((Get-Command php).Source -eq (Join-Path $shimDir 'php.exe')) 'php resolves to the shim in this shell'
# The shim folder carries a copy of the CLI too, which finds the app through app-location.txt.
$cliCopy = Join-Path $shimDir 'lampo.exe'
Check ((Get-Command lampo).Source -eq $cliCopy) 'lampo resolves to the CLI copy in the shim folder'
Check ((Get-Content (Join-Path $DataRoot 'data\app-location.txt') -TotalCount 1) -eq $BinDir) 'the app recorded its folder in app-location.txt'
$copyShims = & $cliCopy shims status --json | ConvertFrom-Json
Check ($LASTEXITCODE -eq 0 -and $copyShims.installed) 'the CLI copy finds the app folder (shims status from the shim folder)'

# --- 3. projects -------------------------------------------------------------------------------

Step 'create two projects on different PHP versions (no elevation)'
$probe = '<?php header("Content-Type: text/plain"); echo "site=", basename(__DIR__), " php=", PHP_VERSION, " route=", $_GET["route"] ?? "-", " code=", is_file(__DIR__ . "/marker.txt") ? trim(file_get_contents(__DIR__ . "/marker.txt")) : "main", PHP_EOL;'
$htaccess = "RewriteEngine On`r`nRewriteCond %{REQUEST_FILENAME} !-f`r`nRewriteRule ^pretty/(.*)$ index.php?route=`$1 [L,QSA]`r`n"
foreach ($n in 'alpha', 'beta') {
    New-Item -ItemType Directory -Force (Join-Path $sites $n) | Out-Null
    Set-Content -Encoding ascii (Join-Path $sites "$n\index.php") $probe
    [IO.File]::WriteAllText((Join-Path $sites "$n\.htaccess"), $htaccess)
}
Set-Content -Encoding ascii (Join-Path $sites 'cli-probe.php') '<?php echo PHP_VERSION, "|", php_ini_loaded_file(), PHP_EOL;'
Push-Location (Join-Path $sites 'beta')
& git init -q -b main 2>$null; & git add -A; & git -c user.email=e2e@lampo.invalid -c user.name=e2e commit -qm init
Pop-Location

$alpha = LampoJson project create (Join-Path $sites 'alpha') --php $newMinor --activate
$beta = LampoJson project create (Join-Path $sites 'beta') --php 7.4 --activate
Check ($alpha.environment.phpVersion -eq $newMinor -and $beta.environment.phpVersion -eq '7.4') "alpha on PHP $newMinor, beta on PHP 7.4"
Check (@($alpha.addresses | Where-Object { $_.kind -eq 'loopback-port' }).Count -eq 1) 'alpha got a loopback port'
$alphaPort = ($alpha.urls | Where-Object { $_ -like 'http://127.0.0.1:*' })
$betaPort = ($beta.urls | Where-Object { $_ -like 'http://127.0.0.1:*' })

# --- 4. HTTP -----------------------------------------------------------------------------------

Step 'fetch a page of each project over HTTP'
foreach ($case in @(@('alpha', $alphaPort, $newMinor), @('beta', $betaPort, '7.4'))) {
    $name, $loopback, $minor = $case
    $body = Http "$loopback/"
    Check ($body -like "site=$name php=$minor.* route=- code=main") "$loopback -> $body"
    $body = Http "http://$name.localhost/pretty/hello-$name"
    Check ($body -like "site=$name php=$minor.* route=hello-$name code=main") "http://$name.localhost/pretty/hello-$name -> $body (.htaccess rewrite)"
}

# --- 5. shims ----------------------------------------------------------------------------------

Step 'php and composer through the shims resolve per folder'
$cliProbe = Join-Path $sites 'cli-probe.php'
foreach ($case in @(@('alpha', $newMinor), @('beta', '7.4'))) {
    $name, $minor = $case
    $dir = Join-Path $sites $name
    $out = (In $dir { php $cliProbe }).Trim()
    Check ($out -like "$minor.*|*\generated\php\$name\php.ini") "php in $name -> $out"
    $v = (In $dir { php -v }).Trim()
    Check ($v -like "PHP $minor.*") "php -v in $name -> $(($v -split "`n")[0])"
    # Composer prints the PHP line on stderr.
    $c = In $dir { composer --version --no-ansi 2>&1 | ForEach-Object { "$_" } }
    Check ($c -match 'Composer version' -and $c -match "PHP version $([regex]::Escape($minor))\.") "composer --version in $name runs on PHP $minor"
}
$out = (In $DataRoot { php $cliProbe }).Trim()
Check ($out -like "$newMinor.*|*\generated\php\_default\php.ini") "php outside projects uses the default PHP -> $out"

# --- 6. database -------------------------------------------------------------------------------

Step 'the database created with the project, queried through the mysql shim'
$betaDir = Join-Path $sites 'beta'
Check ($beta.databaseSetup.status -eq 'created') "project create created the beta database -> $($beta.databaseSetup | ConvertTo-Json -Compress)"
$seed = Join-Path $sites 'seed.sql'
Set-Content -Encoding ascii $seed "CREATE TABLE notes (id INT PRIMARY KEY, body VARCHAR(50)); INSERT INTO notes VALUES (1, 'from-main');"
$null = In $betaDir { Get-Content $seed | mysql beta }
$rows = In $betaDir { mysql --batch --skip-column-names beta -e 'SELECT body FROM notes' }
Check ($rows.Trim() -eq 'from-main') "mysql beta -> $($rows.Trim())"
$dbs = In $betaDir { & $lampo db list --json } | ConvertFrom-Json
Check (@($dbs.databases) -contains 'beta' -or ($dbs | Out-String) -match 'beta') 'lampo db list shows beta'

# --- 7. worktree -------------------------------------------------------------------------------

Step 'git worktree with its own environment and cloned database'
$wtDir = Join-Path $sites 'beta-feature'
Push-Location $betaDir; & git worktree add -q $wtDir -b feature 2>$null; Pop-Location
Set-Content -Encoding ascii (Join-Path $wtDir 'marker.txt') 'feature-x'
$wt = In $wtDir { & $lampo worktree up --db clone:beta --json } | ConvertFrom-Json
Check ($wt.created -and $wt.environment.site -eq 'beta@feature') "environment $($wt.environment.site) at $($wt.environment.url)"
$body = Http "$($wt.environment.url)/pretty/wt"
Check ($body -like 'site=beta-feature php=7.4.* route=wt code=feature-x') "$($wt.environment.url)/pretty/wt -> $body"
$body = Http "$($wt.environment.urls[1])/"
Check ($body -like '*code=feature-x') "$($wt.environment.urls[1]) -> $body"
Check ((Http "$betaPort/") -like '*code=main') 'the parent project still serves its own code'
$out = (In $wtDir { php $cliProbe }).Trim()
Check ($out -like '7.4.*|*\generated\php\beta@feature\php.ini') "php in the worktree -> $out"
$rows = In $wtDir { mysql --batch --skip-column-names $wt.environment.database.database -e 'SELECT body FROM notes' }
Check ($rows.Trim() -eq 'from-main') "cloned database $($wt.environment.database.database) -> $($rows.Trim())"
$again = In $wtDir { & $lampo worktree up --json } | ConvertFrom-Json
Check (-not $again.created) 'worktree up is idempotent'

# --- 8. agent context --------------------------------------------------------------------------

Step 'lampo agent context --json in the project and in the worktree'
$ctx = In $betaDir { & $lampo agent context --json } | ConvertFrom-Json
Check ($ctx.resolution.site -eq 'beta' -and $ctx.resolution.kind -eq 'project' -and $ctx.php.version -eq '7.4' -and $ctx.database.database -eq 'beta' -and $ctx.appRunning) "project: site=$($ctx.resolution.site) url=$($ctx.url) db=$($ctx.database.database)"
$ctx = In $wtDir { & $lampo agent context --json } | ConvertFrom-Json
Check ($ctx.resolution.site -eq 'beta@feature' -and $ctx.resolution.kind -eq 'worktree-environment' -and $ctx.url -eq $wt.environment.url -and $ctx.database.database -eq $wt.environment.database.database -and $ctx.worktree.environmentUp) "worktree: site=$($ctx.resolution.site) url=$($ctx.url) db=$($ctx.database.database)"
Check (@($ctx.commands | Where-Object { $_.name -eq 'lampo quit' }).Count -eq 1) 'the command catalog lists lampo quit'

# --- 9. logs -----------------------------------------------------------------------------------

Step 'lampo logs shows the requests'
$log = In $wtDir { & $lampo logs access -n 50 --json } | ConvertFrom-Json
Check (@($log.lines | Where-Object { $_.text -match 'GET /pretty/wt ' }).Count -ge 1) "access log of beta@feature has GET /pretty/wt ($($log.file))"
$log = In $betaDir { & $lampo logs access -n 50 --json } | ConvertFrom-Json
Check (@($log.lines | Where-Object { $_.text -match 'GET /pretty/hello-beta ' }).Count -ge 1) 'access log of beta has GET /pretty/hello-beta'

# --- 10. down and quit -------------------------------------------------------------------------

Step 'lampo down, then lampo quit, leave nothing behind'
Check ($null -ne (Service-Processes)) "services running: $((Service-Processes | ForEach-Object { $_.ProcessName }) -join ', ')"
$null = Lampo down
Check (Wait-Until { $null -eq (Service-Processes) }) 'no httpd/php-cgi/mysqld/redis process after lampo down'
$null = Lampo quit
Check (Wait-Until { $null -eq (App-Processes) }) 'the app exited after lampo quit'
Check ($null -eq (Stack-Processes)) 'no process of the stack is left'

# --- 11. hard kill -----------------------------------------------------------------------------

Step 'lampo up, then kill the app hard: services die with it (Job Object)'
$null = Lampo up
Check ((Http "$alphaPort/") -like 'site=alpha*') 'served again after lampo up (app restarted headless)'
Check ($null -ne (Service-Processes)) "services running: $((Service-Processes | ForEach-Object { $_.ProcessName }) -join ', ')"
if ($Keep) { Write-Host "`n-Keep: leaving the app running. Data root: $DataRoot"; exit 0 }
App-Processes | Stop-Process -Force
Check (Wait-Until { $null -eq (Stack-Processes) } 10) 'no httpd/php-cgi/mysqld/redis process after killing lampo-desktop.exe'

Write-Host "`nAll steps passed. Data root: $DataRoot" -ForegroundColor Green
exit 0
