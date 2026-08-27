#requires -version 5
$ErrorActionPreference = 'SilentlyContinue'
$app    = 'Battery Hub'
$target = Join-Path $env:LOCALAPPDATA $app

Get-Process battery-hub | Stop-Process -Force
Start-Sleep -Milliseconds 800

Remove-Item (Join-Path $target 'battery-hub.exe') -Force
Remove-Item (Join-Path $target 'WebView2Loader.dll') -Force
$shell = New-Object -ComObject WScript.Shell
Remove-Item (Join-Path $shell.SpecialFolders('Programs') "$app.lnk") -Force
Remove-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Battery Hub' -Recurse -Force
Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name $app -Force

Write-Host "Kaldirildi. devices.json ve diagnostics.log $target icinde birakildi." -ForegroundColor Yellow
Write-Host 'Ayarlar: %APPDATA%\com.axthrowa.battery-hub' -ForegroundColor Yellow
