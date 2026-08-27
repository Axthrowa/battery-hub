#requires -version 5
$ErrorActionPreference = 'SilentlyContinue'
$app    = 'Battery Hub'
$target = Join-Path $env:LOCALAPPDATA $app
$store  = Join-Path $env:APPDATA 'com.axthrowa.battery-hub'

Get-Process battery-hub | Stop-Process -Force
Start-Sleep -Milliseconds 800

Remove-Item (Join-Path $target 'battery-hub.exe') -Force
Remove-Item (Join-Path $target 'onceki-battery-hub.exe') -Force
Remove-Item (Join-Path $target 'WebView2Loader.dll') -Force
Remove-Item (Join-Path $target 'diagnostics.log') -Force
$shell = New-Object -ComObject WScript.Shell
Remove-Item (Join-Path $shell.SpecialFolders('Programs') "$app.lnk") -Force
Remove-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Battery Hub' -Recurse -Force
Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name $app -Force

# The settings, the taught devices, the pictures and the sounds are the user's
# work, not the program's. Ask; do not assume.
Write-Host ''
Write-Host 'Ayarlar, eklenen cihazlar, kart gorselleri ve bildirim sesleri de silinsin mi?' -ForegroundColor Cyan
Write-Host 'Hayir derseniz yeniden kurdugunuzda hepsi yerinde olur.' -ForegroundColor DarkGray
$answer = Read-Host 'Silinsin mi? (E/H)'
if ($answer -match '^[EeYy]') {
    Remove-Item (Join-Path $target 'devices.json') -Force
    Remove-Item (Join-Path $target 'notification-full.wav') -Force
    Remove-Item (Join-Path $target 'notification-low.wav') -Force
    Remove-Item $store -Recurse -Force
    Write-Host 'Kullanici verileri de silindi.' -ForegroundColor Yellow
} else {
    Write-Host "Veriler birakildi: $target" -ForegroundColor Yellow
    Write-Host "                   $store" -ForegroundColor Yellow
}

Remove-Item $target -Force   # only succeeds once it is empty
Write-Host 'Kaldirildi.' -ForegroundColor Green
