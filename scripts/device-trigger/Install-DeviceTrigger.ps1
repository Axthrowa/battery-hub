<#
.SYNOPSIS
  Registers a scheduled task that starts Battery Hub when a device is
  attached, so the app comes back after the 2.4 GHz dongle is re-plugged.

.DESCRIPTION
  The app exits completely when the dongle is removed, which means nothing is
  left running to notice it coming back. Windows raises a Kernel-PnP
  "device started" event (410) on re-plug, and Task Scheduler can start us from
  it.

  The event-log XPath subset has no contains(), so the subscription cannot be
  filtered down to Razer's VID. Instead the task passes --require-dongle and the
  app enumerates HID once and exits in milliseconds when the device that
  arrived was not ours. Running only while a user is logged on also keeps the
  boot-time burst of 410 events from triggering it.

.PARAMETER ExePath
  Full path to battery-hub.exe. Defaults to the standard install dir.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\Install-DeviceTrigger.ps1
#>
[CmdletBinding()]
param(
    [string] $ExePath = "$env:ProgramFiles\Battery Hub\battery-hub.exe",
    [string] $TaskName = 'Battery Hub - Device Arrival'
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $ExePath)) {
    throw "battery-hub.exe not found at '$ExePath'. Pass -ExePath if you installed elsewhere."
}

$subscription = @'
<QueryList><Query Id="0" Path="Microsoft-Windows-Kernel-PnP/Configuration"><Select Path="Microsoft-Windows-Kernel-PnP/Configuration">*[System[(EventID=410)]]</Select></Query></QueryList>
'@

$class = cimclass MSFT_TaskEventTrigger root/Microsoft/Windows/TaskScheduler
$trigger = $class | New-CimInstance -ClientOnly
$trigger.Enabled = $true
$trigger.Subscription = $subscription
# Let the HID stack finish enumerating before we go looking for the dongle.
$trigger.Delay = 'PT5S'

$action = New-ScheduledTaskAction -Execute $ExePath -Argument '--require-dongle'

# InteractiveToken: the app must land in the logged-on user's session or its
# tray icon and toasts would go to session 0 where nobody can see them.
$principal = New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) `
                                        -LogonType Interactive `
                                        -RunLevel Limited

# ExecutionTimeLimit must be zero: this task starts a long-running tray app,
# and any limit would have Task Scheduler kill it later.
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries `
                                         -DontStopIfGoingOnBatteries `
                                         -MultipleInstances IgnoreNew `
                                         -ExecutionTimeLimit ([TimeSpan]::Zero) `
                                         -StartWhenAvailable:$false

Register-ScheduledTask -TaskName $TaskName `
                       -Trigger $trigger `
                       -Action $action `
                       -Principal $principal `
                       -Settings $settings `
                       -Description 'Starts Battery Hub when a wireless receiver is plugged in.' `
                       -Force | Out-Null

Write-Host "Registered scheduled task '$TaskName'." -ForegroundColor Green
Write-Host "Target: $ExePath --require-dongle"
