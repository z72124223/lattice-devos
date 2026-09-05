[CmdletBinding()]
param(
    [string]$TaskName = "LATTICE Control ($env:USERNAME)",
    [string]$NodePath = '',
    [string]$DatabasePath = (Join-Path $env:LOCALAPPDATA 'LATTICE\control\lattice-control.db'),
    [ValidateRange(1024, 65535)][int]$Port = 4317,
    [ValidatePattern('^$|^[0-9a-f]{64}$')][string]$ExpectedPreviousTaskSha256 = '',
    [switch]$NoStart
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$runnerPath = Join-Path $PSScriptRoot 'Run-LatticeControlBackgroundTask.ps1'
$serverPath = Join-Path $repositoryRoot 'apps\lattice-control\src\server.mjs'
if (-not (Test-Path -LiteralPath $serverPath -PathType Leaf)) {
    throw 'LATTICE_CONTROL_SERVER_MISSING'
}
if ([string]::IsNullOrWhiteSpace($NodePath)) {
    $NodePath = @(Get-Command node.exe -CommandType Application -ErrorAction Stop)[0].Source
}
$NodePath = (Resolve-Path -LiteralPath $NodePath -ErrorAction Stop).Path
$DatabasePath = [IO.Path]::GetFullPath($DatabasePath)
$nodeVersion = & $NodePath --version
if ($LASTEXITCODE -ne 0 -or $nodeVersion -notmatch '^v(\d+\.\d+\.\d+)$' -or [version]$Matches[1] -lt [version]'24.15.0') {
    throw 'LATTICE_CONTROL_NODE_24_15_REQUIRED'
}

function Quote-TaskArgument([string]$Value) {
    if ($Value -match '["\r\n\x00]') { throw 'LATTICE_CONTROL_TASK_ARGUMENT_REJECTED' }
    return '"' + $Value + '"'
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$taskDescription = 'LATTICE Control per-user background service v1; Windows owns its lifetime.'
$powershellPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$arguments = '-NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -File ' + (Quote-TaskArgument $runnerPath) +
    ' -NodePath ' + (Quote-TaskArgument $NodePath) +
    ' -DatabasePath ' + (Quote-TaskArgument $DatabasePath) + ' -Port ' + $Port
$action = New-ScheduledTaskAction -Execute $powershellPath -Argument $arguments -WorkingDirectory $repositoryRoot
$principal = New-ScheduledTaskPrincipal -UserId $identity.User.Value -LogonType Interactive -RunLevel Limited
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $identity.User.Value
# RestartOnFailure does not reliably recover a child that already started and exited.
# Windows periodically starts this same task; IgnoreNew leaves a running server alone.
$recoveryTrigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) -RepetitionInterval (New-TimeSpan -Minutes 1)
$settings = New-ScheduledTaskSettingsSet -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero) `
    -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -StartWhenAvailable `
    -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
$existing = Get-ScheduledTask -TaskName $TaskName -TaskPath '\' -ErrorAction SilentlyContinue
$needsRegistration = $true
if ($null -ne $existing) {
    $existingOwner = [string]$existing.Principal.UserId
    if ($existingOwner -notmatch '^S-1-') {
        $existingOwner = [Security.Principal.NTAccount]::new($existingOwner).Translate([Security.Principal.SecurityIdentifier]).Value
    }
    $sameOwner = $existingOwner -eq $identity.User.Value
    if (-not $sameOwner -or $existing.Description -ne $taskDescription -or
        @($existing.Actions).Count -ne 1 -or $existing.Actions[0].Execute -ne $powershellPath) {
        throw 'LATTICE_CONTROL_EXISTING_TASK_SCOPE_MISMATCH'
    }
    $sameScope = $existing.Actions[0].Arguments -eq $arguments -and $existing.Actions[0].WorkingDirectory -eq $repositoryRoot
    if (-not $sameScope) {
        # A version migration must bind the exact stopped task previously inspected by the installer.
        if ([string]::IsNullOrEmpty($ExpectedPreviousTaskSha256) -or $existing.State -eq 'Running') {
            throw 'LATTICE_CONTROL_EXISTING_TASK_SCOPE_MISMATCH'
        }
        $previousXml = Export-ScheduledTask -TaskName $TaskName -TaskPath '\'
        $hashAlgorithm = [Security.Cryptography.SHA256]::Create()
        try {
            $previousHash = ([BitConverter]::ToString($hashAlgorithm.ComputeHash([Text.Encoding]::UTF8.GetBytes($previousXml)))).Replace('-', '').ToLowerInvariant()
        } finally { $hashAlgorithm.Dispose() }
        if ($previousHash -cne $ExpectedPreviousTaskSha256) {
            throw 'LATTICE_CONTROL_PREVIOUS_TASK_CHANGED'
        }
    }
    $logonTriggers = @($existing.Triggers | Where-Object { $_.CimClass.CimClassName -eq 'MSFT_TaskLogonTrigger' -and $_.Enabled })
    $recoveryTriggers = @($existing.Triggers | Where-Object {
        $_.CimClass.CimClassName -eq 'MSFT_TaskTimeTrigger' -and $_.Enabled -and
        $_.Repetition.Interval -eq 'PT1M' -and [string]::IsNullOrEmpty($_.Repetition.Duration) -and
        [string]::IsNullOrEmpty($_.EndBoundary)
    })
    $needsRegistration = -not ($sameScope -and $existing.Principal.LogonType -eq 'Interactive' -and
        $existing.Principal.RunLevel -eq 'Limited' -and $existing.Settings.Enabled -and
        $existing.Settings.MultipleInstances -eq 'IgnoreNew' -and $existing.Settings.ExecutionTimeLimit -eq 'PT0S' -and
        $existing.Settings.RestartCount -eq 3 -and $existing.Settings.RestartInterval -eq 'PT1M' -and
        -not $existing.Settings.DisallowStartIfOnBatteries -and -not $existing.Settings.StopIfGoingOnBatteries -and
        $existing.Settings.StartWhenAvailable -and @($existing.Triggers).Count -eq 2 -and
        $logonTriggers.Count -eq 1 -and $recoveryTriggers.Count -eq 1)
    if ($needsRegistration -and $existing.State -eq 'Running') {
        throw 'LATTICE_CONTROL_STOP_OWNED_TASK_BEFORE_CHANGING_SETTINGS'
    }
}
if ($needsRegistration) {
    $registered = Register-ScheduledTask -TaskName $TaskName -TaskPath '\' -Description $taskDescription `
        -Action $action -Principal $principal -Trigger @($trigger, $recoveryTrigger) -Settings $settings -Force
} else {
    $registered = $existing
}
if (-not $NoStart -and $registered.State -ne 'Running') {
    Start-ScheduledTask -TaskName $TaskName -TaskPath '\'
}
[pscustomobject]@{
    task_name = $TaskName
    state = [string](Get-ScheduledTask -TaskName $TaskName -TaskPath '\').State
    owner = $identity.Name
    node_path = $NodePath
    server_path = $serverPath
    database_path = $DatabasePath
    origin = "http://127.0.0.1:$Port"
    logon_start = $true
    startup_failure_retries = 3
    recovery_trigger_interval_seconds = 60
    registration_changed = $needsRegistration
    requires_logged_in_user = $true
} | ConvertTo-Json
