[CmdletBinding()]
param(
    [string]$TaskName = "LATTICE Control ($env:USERNAME)",
    [ValidateRange(1024, 65535)][int]$Port = 4317,
    [switch]$CrashRecovery
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$serverPath = Join-Path $repositoryRoot 'apps\lattice-control\src\server.mjs'
$runnerPath = Join-Path $PSScriptRoot 'Run-LatticeControlBackgroundTask.ps1'
$origin = "http://127.0.0.1:$Port"
function Assert-True($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Get-OwnedListener {
    $listeners = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction Stop)
    Assert-True ($listeners.Count -eq 1 -and $listeners[0].LocalAddress -eq '127.0.0.1') 'Loopback listener required'
    $server = Get-CimInstance Win32_Process -Filter "ProcessId = $($listeners[0].OwningProcess)"
    Assert-True ($null -ne $server -and $server.Name -eq 'node.exe' -and $server.CommandLine.Contains($serverPath)) 'Unexpected listener owner'
    $runner = Get-CimInstance Win32_Process -Filter "ProcessId = $($server.ParentProcessId)"
    Assert-True ($null -ne $runner -and $runner.Name -eq 'powershell.exe' -and $runner.CommandLine.Contains($runnerPath)) 'Unexpected launcher'
    $scheduler = @(Get-CimInstance Win32_Service -Filter "ProcessId = $($runner.ParentProcessId)" | Where-Object Name -eq 'Schedule')
    Assert-True ($scheduler.Count -eq 1) 'Control must be owned by Windows Task Scheduler'
    return $server
}

$task = Get-ScheduledTask -TaskName $TaskName -TaskPath '\'
Assert-True ($task.State -eq 'Running') 'Task is not running'
Assert-True ($task.Principal.LogonType -eq 'Interactive' -and $task.Principal.RunLevel -eq 'Limited') 'Unexpected task authority'
Assert-True ($task.Settings.MultipleInstances -eq 'IgnoreNew' -and $task.Settings.ExecutionTimeLimit -eq 'PT0S') 'Unexpected lifetime settings'
Assert-True ($task.Settings.RestartCount -eq 3 -and $task.Settings.RestartInterval -eq 'PT1M') 'Unexpected recovery settings'
Assert-True (-not $task.Settings.DisallowStartIfOnBatteries -and -not $task.Settings.StopIfGoingOnBatteries) 'Battery would stop Control'
Assert-True (@($task.Triggers | Where-Object { $_.CimClass.CimClassName -eq 'MSFT_TaskLogonTrigger' -and $_.Enabled }).Count -eq 1) 'Logon trigger missing'
Assert-True (@($task.Triggers | Where-Object {
    $_.CimClass.CimClassName -eq 'MSFT_TaskTimeTrigger' -and $_.Enabled -and
    $_.Repetition.Interval -eq 'PT1M' -and [string]::IsNullOrEmpty($_.Repetition.Duration)
}).Count -eq 1) 'Continuous Windows recovery trigger missing'
$before = Get-OwnedListener
$state = Invoke-RestMethod -Uri "$origin/api/state" -TimeoutSec 10
$projectIds = @($state.projects.id | Sort-Object) -join ','
Start-ScheduledTask -TaskName $TaskName -TaskPath '\'
$afterDuplicate = Get-OwnedListener
Assert-True ($before.ProcessId -eq $afterDuplicate.ProcessId) 'Duplicate start replaced the running listener'

$report = [ordered]@{
    result = 'PASS'
    windows_owned = $true
    loopback_only = $true
    settings_verified = $true
    duplicate_start_preserved_process = $true
    original_server_pid = $before.ProcessId
    crash_recovery = 'NOT_RUN'
}
if ($CrashRecovery) {
    $conversation = Invoke-RestMethod -Uri "$origin/api/conversation" -TimeoutSec 10
    Assert-True (@($state.workItems).Count -eq 0 -and -not $conversation.can_interrupt -and
        $conversation.status -in @('idle', 'codex_done')) 'Refusing to interrupt active or uncertain work'
    $ownedProcess = Get-Process -Id $before.ProcessId -ErrorAction Stop
    $ownedHandle = $ownedProcess.Handle
    Assert-True ($ownedProcess.Path -eq $before.ExecutablePath -and
        [Math]::Abs(($ownedProcess.StartTime - $before.CreationDate).TotalMilliseconds) -lt 1) 'Process identity changed'
    $faultTime = [DateTime]::UtcNow
    $ownedProcess.Kill()
    $ownedProcess.WaitForExit()
    $ownedProcess.Dispose()
    Write-Output 'Controlled idle-server failure injected; waiting for Windows recovery.'
    $recovered = $null
    while ([DateTime]::UtcNow -lt $faultTime.AddSeconds(100)) {
        Start-Sleep -Seconds 2
        try {
            $candidate = Get-OwnedListener
            if ($candidate.ProcessId -ne $before.ProcessId) {
                $afterState = Invoke-RestMethod -Uri "$origin/api/state" -TimeoutSec 3
                $recovered = $candidate
                break
            }
        } catch { continue }
    }
    Assert-True ($null -ne $recovered) 'Windows recovery did not restore Control within 100 seconds'
    Assert-True ((@($afterState.projects.id | Sort-Object) -join ',') -eq $projectIds) 'Project catalog changed after recovery'
    $report.crash_recovery = 'PASS'
    $report.recovered_server_pid = $recovered.ProcessId
    $report.recovery_seconds = [Math]::Round(([DateTime]::UtcNow - $faultTime).TotalSeconds, 2)
    $report.project_catalog_preserved = $true
}
$report | ConvertTo-Json
