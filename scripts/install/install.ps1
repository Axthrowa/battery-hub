#requires -version 5
# Battery Hub kurulumu. Smart App Control imzali EXE'lere izin veriyor ama
# imzali NSIS setup'ini da engelledigi icin kurulum bu script ile yapiliyor.
$ErrorActionPreference = 'Stop'
$here   = Split-Path -Parent $MyInvocation.MyCommand.Path
$app    = 'Battery Hub'
$target = Join-Path $env:LOCALAPPDATA $app
$exe    = Join-Path $target 'battery-hub.exe'

Write-Host "Battery Hub -> $target" -ForegroundColor Cyan

Get-Process battery-hub -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800

New-Item -ItemType Directory -Path $target -Force | Out-Null
Copy-Item (Join-Path $here 'battery-hub.exe')    $exe -Force
Copy-Item (Join-Path $here 'WebView2Loader.dll') (Join-Path $target 'WebView2Loader.dll') -Force
Copy-Item (Join-Path $here 'Kaldir.cmd')         (Join-Path $target 'Kaldir.cmd') -Force
Copy-Item (Join-Path $here 'uninstall.ps1')      (Join-Path $target 'uninstall.ps1') -Force

$shell = New-Object -ComObject WScript.Shell
$lnk = $shell.CreateShortcut((Join-Path $shell.SpecialFolders('Programs') "$app.lnk"))
$lnk.TargetPath = $exe
$lnk.WorkingDirectory = $target
$lnk.IconLocation = "$exe,0"
$lnk.Save()

$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Battery Hub'
New-Item -Path $key -Force | Out-Null
Set-ItemProperty $key DisplayName     $app
Set-ItemProperty $key DisplayVersion  '0.1.0'
Set-ItemProperty $key Publisher       'Axthrowa'
Set-ItemProperty $key DisplayIcon     $exe
Set-ItemProperty $key InstallLocation $target
Set-ItemProperty $key UninstallString ('"{0}"' -f (Join-Path $target 'Kaldir.cmd'))
Set-ItemProperty $key NoModify 1 -Type DWord
Set-ItemProperty $key NoRepair 1 -Type DWord

# Windows kabuk ikon onbellegini tazele
Add-Type -Namespace Shell32 -Name Api -MemberDefinition @'
[DllImport("shell32.dll")] public static extern void SHChangeNotify(int e, uint f, IntPtr a, IntPtr b);
'@
[Shell32.Api]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)

$sig = (Get-AuthenticodeSignature $exe).Status
Write-Host "Kuruldu. Imza: $sig" -ForegroundColor Green
Write-Host "Baslat menusunde 'Battery Hub' olarak gorunur." -ForegroundColor Green

if ((Read-Host 'Simdi baslatilsin mi? (E/h)') -notmatch '^[hH]') {
    Start-Process $exe
    Write-Host 'Tepside calisiyor.' -ForegroundColor Green
}
