<#
.SYNOPSIS
  Removes the scheduled task registered by Install-DeviceTrigger.ps1.
#>
[CmdletBinding()]
param([string] $TaskName = 'Battery Hub - Device Arrival')

$ErrorActionPreference = 'Stop'

if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
    Write-Host "Removed scheduled task '$TaskName'." -ForegroundColor Green
} else {
    Write-Host "No scheduled task named '$TaskName'." -ForegroundColor Yellow
}
