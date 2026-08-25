# Build a one-file tray executable.
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$python = if (Test-Path ".\.venv\Scripts\python.exe") {
    ".\.venv\Scripts\python.exe"
} elseif (Get-Command python -ErrorAction SilentlyContinue) {
    "python"
} else {
    throw "python bulunamadi. https://www.python.org/downloads/ adresinden 3.10+ kurun (Add to PATH)."
}

if (-not (Test-Path ".\.venv\Scripts\python.exe")) {
    & $python -m venv .venv
    $python = ".\.venv\Scripts\python.exe"
}

& $python -m pip install --upgrade pip
& $python -m pip install -r requirements.txt

$entry = Join-Path $PSScriptRoot "run_app.py"
@"
from blackshark_battery.app import main
raise SystemExit(main())
"@ | Set-Content -Path $entry -Encoding UTF8

& $python -m PyInstaller --noconsole --onefile --clean `
    --name BlackSharkBattery `
    --hidden-import=blackshark_battery `
    --hidden-import=blackshark_battery.app `
    --hidden-import=blackshark_battery.hid_source `
    --hidden-import=blackshark_battery.bluetooth_source `
    --hidden-import=blackshark_battery.settings `
    --hidden-import=pystray._win32 `
    --hidden-import=pystray._util.win32 `
    --collect-all hid `
    --collect-submodules blackshark_battery `
    $entry

Write-Host "OK: $PSScriptRoot\dist\BlackSharkBattery.exe"
