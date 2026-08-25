<#
.SYNOPSIS
  Signs the portable battery-hub.exe produced by `npm run tauri:build`.

.DESCRIPTION
  Tauri signs the executable before packing it into the NSIS installer and then
  restores the unsigned original on disk, so the portable binary needs one more
  pass. The installer itself is already signed by the build. The thumbprint
  comes from src-tauri/tauri.local.conf.json (gitignored, machine-specific).
#>
param(
    [string] $ExePath,
    [string] $TimestampUrl = 'http://timestamp.digicert.com'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $ExePath) {
    $ExePath = Join-Path $root 'src-tauri\target\release\battery-hub.exe'
}

# The certificate is machine-specific, so it lives in a gitignored local
# config that is merged into the build via --config; fall back to the main
# config for setups that keep the thumbprint there.
$thumbprint = $null
foreach ($name in 'tauri.local.conf.json', 'tauri.conf.json') {
    $path = Join-Path $root "src-tauri\$name"
    if (-not (Test-Path -LiteralPath $path)) { continue }
    $conf = Get-Content $path -Raw | ConvertFrom-Json
    $thumbprint = $conf.bundle.windows.certificateThumbprint
    if ($thumbprint) { break }
}
if (-not $thumbprint) {
    throw 'No certificateThumbprint found - copy src-tauri/tauri.local.conf.json.example to tauri.local.conf.json and fill it in.'
}
if (-not (Test-Path -LiteralPath $ExePath)) {
    throw "Not found: $ExePath - run 'npm run tauri:build' first."
}

$signtool = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe' -ErrorAction SilentlyContinue |
    Sort-Object FullName | Select-Object -Last 1
if (-not $signtool) {
    throw 'signtool.exe not found - install the Windows 10/11 SDK.'
}

& $signtool.FullName sign /sha1 $thumbprint /fd sha256 /tr $TimestampUrl /td sha256 $ExePath
if ($LASTEXITCODE -ne 0) { throw "signtool sign failed ($LASTEXITCODE)" }
& $signtool.FullName verify /pa $ExePath
if ($LASTEXITCODE -ne 0) { throw "signtool verify failed ($LASTEXITCODE)" }
