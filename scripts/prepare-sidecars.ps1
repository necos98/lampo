<#
.SYNOPSIS
  Builds the executables that ship next to lampo-desktop.exe and stages them as Tauri external
  binaries (`apps\desktop\src-tauri\binaries\<name>-<target-triple>.exe`).

.DESCRIPTION
  The installer places them next to the app executable, with the target triple removed:
    lampo.exe               the CLI (finds lampo-desktop.exe next to itself to start it headless;
                            its copy in the shim dir reads <data>\app-location.txt instead)
    lampo-shim.exe          copied by Lampo as php.exe, composer.exe, mysql.exe... into the shim dir
    lampo-hosts-helper.exe  the elevated helper: hosts-file writer and, on request, the Defender
                            exclusion of the MySQL data folder
  The app and the CLI look for these in their own folder (`composition.rs` of each).

  Used by `pnpm bundle` (`tauri build --config src-tauri/tauri.bundle.conf.json`). During
  development nothing is needed: every binary is built into target\debug next to the app.

.PARAMETER Profile
  Cargo profile to build (default: release).
#>
param([string]$Profile = 'release')

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$triple = ((& rustc -vV) | Where-Object { $_ -like 'host: *' }) -replace '^host: ', ''
if (-not $triple) { throw 'cannot determine the Rust host target triple (rustc -vV)' }

$profileArgs = if ($Profile -eq 'debug') { @() } else { @('--profile', $Profile) }
& cargo build --manifest-path (Join-Path $root 'Cargo.toml') -p lampo-cli -p lampo-infra-web --bins --locked @profileArgs
if ($LASTEXITCODE -ne 0) { throw "cargo build failed with $LASTEXITCODE" }

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
$outDir = Join-Path $targetDir $Profile
$binaries = Join-Path $root 'apps\desktop\src-tauri\binaries'
New-Item -ItemType Directory -Force $binaries | Out-Null
foreach ($name in 'lampo', 'lampo-shim', 'lampo-hosts-helper') {
    $source = Join-Path $outDir "$name.exe"
    if (-not (Test-Path $source)) { throw "$source was not built" }
    Copy-Item $source (Join-Path $binaries "$name-$triple.exe") -Force
    Write-Host "staged $name-$triple.exe"
}
