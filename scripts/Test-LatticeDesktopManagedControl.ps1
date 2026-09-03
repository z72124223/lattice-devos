[CmdletBinding()]
param(
    [string]$CandidateArchive = '',
    [ValidateRange(10, 120)]
    [int]$TimeoutSeconds = 45
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Text.UTF8Encoding]::new($false)

function Get-Sha256Hex {
    param([string]$LiteralPath)

    $stream = [IO.File]::Open(
        [IO.Path]::GetFullPath($LiteralPath),
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::Read)
    try {
        $algorithm = [Security.Cryptography.SHA256]::Create()
        try {
            return ([BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
        }
        finally { $algorithm.Dispose() }
    }
    finally { $stream.Dispose() }
}

function Assert-CandidateProcessIdentity {
    param(
        [Diagnostics.Process]$DesktopProcess,
        [string]$ExpectedExecutablePath,
        [string]$ExpectedExecutableSha256,
        [string]$ExpectedSourceCommit
    )

    $identityDeadline = [DateTimeOffset]::UtcNow.AddSeconds(10)
    while ($true) {
        try {
            $DesktopProcess.Refresh()
            if ($DesktopProcess.HasExited) {
                throw "DESKTOP_MANAGED_CONTROL_PROCESS_EXITED:$($DesktopProcess.ExitCode)"
            }
            $observedPath = [IO.Path]::GetFullPath($DesktopProcess.MainModule.FileName)
            break
        }
        catch {
            if ($_.Exception.Message -clike 'DESKTOP_MANAGED_CONTROL_PROCESS_EXITED:*') {
                throw
            }
            try { $DesktopProcess.Refresh() } catch { }
            if ($DesktopProcess.HasExited) {
                throw "DESKTOP_MANAGED_CONTROL_PROCESS_EXITED:$($DesktopProcess.ExitCode)"
            }
            if ([DateTimeOffset]::UtcNow -ge $identityDeadline) {
                throw 'DESKTOP_MANAGED_CONTROL_PROCESS_IDENTITY_UNAVAILABLE'
            }
            Start-Sleep -Milliseconds 50
        }
    }

    $expectedPath = [IO.Path]::GetFullPath($ExpectedExecutablePath)
    if (-not [string]::Equals(
        $observedPath,
        $expectedPath,
        [StringComparison]::OrdinalIgnoreCase)) {
        throw 'DESKTOP_MANAGED_CONTROL_PROCESS_PATH_MISMATCH'
    }
    $observedSha256 = Get-Sha256Hex -LiteralPath $observedPath
    if ($observedSha256 -cne $ExpectedExecutableSha256) {
        throw 'DESKTOP_MANAGED_CONTROL_PROCESS_HASH_MISMATCH'
    }
    $productVersion = [string]([Diagnostics.FileVersionInfo]::GetVersionInfo(
        $observedPath).ProductVersion)
    if (-not $productVersion.EndsWith(
        '+' + $ExpectedSourceCommit,
        [StringComparison]::Ordinal)) {
        throw 'DESKTOP_MANAGED_CONTROL_PROCESS_REVISION_MISMATCH'
    }

    return [PSCustomObject][ordered]@{
        pid = $DesktopProcess.Id
        executable_path = $observedPath
        executable_sha256 = $observedSha256
        source_revision = $ExpectedSourceCommit
        product_version = $productVersion
    }
}

function Test-ProcessAlive {
    param([AllowNull()][Diagnostics.Process]$Process)
    if ($null -eq $Process) { return $false }
    try {
        $Process.Refresh()
        return -not $Process.HasExited
    }
    catch { return $false }
}

function Stop-TestOwnedProcess {
    param([AllowNull()][Diagnostics.Process]$Process)
    if (-not (Test-ProcessAlive $Process)) { return }
    $Process.Kill()
    if (-not $Process.WaitForExit(10000)) {
        throw "DESKTOP_MANAGED_CONTROL_OWNED_PROCESS_STOP_TIMEOUT:$($Process.Id)"
    }
}

function Get-TestBoundProcessIdentity {
    param(
        [Diagnostics.Process]$Process,
        [int]$ExpectedParentProcessId = -1,
        [DateTime]$ExpectedCreationUtc = [DateTime]::MinValue
    )

    $processId = $Process.Id
    [void]$Process.Handle
    $startedAtUtc = $Process.StartTime.ToUniversalTime()
    $records = @(Get-CimInstance Win32_Process -Filter "ProcessId = $processId")
    if ($records.Count -ne 1) {
        throw "DESKTOP_MANAGED_CONTROL_PROCESS_CIM_IDENTITY_UNAVAILABLE:$processId"
    }
    $record = $records[0]
    $creationUtc = ([DateTime]$record.CreationDate).ToUniversalTime()
    if ($ExpectedParentProcessId -ge 0 -and
        [int]$record.ParentProcessId -ne $ExpectedParentProcessId) {
        throw "DESKTOP_MANAGED_CONTROL_PROCESS_PARENT_MISMATCH:$processId"
    }
    if ($ExpectedCreationUtc -ne [DateTime]::MinValue -and
        $creationUtc.Ticks -ne $ExpectedCreationUtc.Ticks) {
        throw "DESKTOP_MANAGED_CONTROL_PROCESS_CIM_GENERATION_MISMATCH:$processId"
    }
    if ([Math]::Abs(($startedAtUtc - $creationUtc).TotalMilliseconds) -gt 1) {
        throw "DESKTOP_MANAGED_CONTROL_PROCESS_START_TIME_MISMATCH:$processId"
    }
    return [PSCustomObject][ordered]@{
        process = $Process
        process_id = $processId
        parent_process_id = [int]$record.ParentProcessId
        started_at_utc = $startedAtUtc
        creation_utc = $creationUtc
    }
}

function Get-TestOwnedDescendantSnapshot {
    param([object[]]$OwnedParents, [DateTime]$LatestOwnedStartUtc)

    $found = [Collections.Generic.List[object]]::new()
    foreach ($ownedParent in $OwnedParents) {
        $parentProcessId = [int]$ownedParent.process_id
        $parentStartedAtUtc = [DateTime]$ownedParent.started_at_utc
        foreach ($childRecord in @(Get-CimInstance Win32_Process -Filter (
            "ParentProcessId = $parentProcessId"))) {
            $childProcessId = [int]$childRecord.ProcessId
            $childCreationUtc = ([DateTime]$childRecord.CreationDate).ToUniversalTime()
            try {
                $childProcess = [Diagnostics.Process]::GetProcessById($childProcessId)
                $childIdentity = Get-TestBoundProcessIdentity `
                    -Process $childProcess `
                    -ExpectedParentProcessId $parentProcessId `
                    -ExpectedCreationUtc $childCreationUtc
            }
            catch {
                throw "DESKTOP_MANAGED_CONTROL_DESCENDANT_IDENTITY_UNAVAILABLE:$childProcessId"
            }
            if ([DateTime]$childIdentity.started_at_utc -lt $parentStartedAtUtc -or
                [DateTime]$childIdentity.started_at_utc -gt $LatestOwnedStartUtc) {
                throw "DESKTOP_MANAGED_CONTROL_DESCENDANT_IDENTITY_AMBIGUOUS:$childProcessId"
            }
            [void]$found.Add($childIdentity)
        }
    }
    return $found.ToArray()
}

function Stop-TestOwnedProcessTree {
    param(
        [Diagnostics.Process]$OwnedRoot,
        [AllowNull()][object]$OwnedRootIdentity,
        [int]$TimeoutMilliseconds = 10000
    )

    $rootProcessId = $OwnedRoot.Id
    $cleanupStartedAt = [DateTimeOffset]::UtcNow
    $deadline = $cleanupStartedAt.AddMilliseconds($TimeoutMilliseconds)
    if ($null -eq $OwnedRootIdentity) {
        while (Test-ProcessAlive $OwnedRoot -and [DateTimeOffset]::UtcNow -lt $deadline) {
            try { $OwnedRoot.Kill() } catch { }
            Start-Sleep -Milliseconds 50
        }
        if (Test-ProcessAlive $OwnedRoot) {
            throw "DESKTOP_MANAGED_CONTROL_OWNED_ROOT_STOP_TIMEOUT:$rootProcessId"
        }
        throw "DESKTOP_MANAGED_CONTROL_OWNED_ROOT_IDENTITY_UNAVAILABLE:$rootProcessId"
    }

    $latestOwnedStartUtc = $cleanupStartedAt.UtcDateTime.AddSeconds(2)
    $knownDescendants = @{}
    $lastDiscoveryFailure = $null
    try {
        foreach ($descendant in @(Get-TestOwnedDescendantSnapshot `
            -OwnedParents @($OwnedRootIdentity) `
            -LatestOwnedStartUtc $latestOwnedStartUtc)) {
            $knownDescendants[[int]$descendant.process_id] = $descendant
        }
    }
    catch { $lastDiscoveryFailure = $_ }
    if (Test-ProcessAlive $OwnedRoot) { try { $OwnedRoot.Kill() } catch { } }

    $quietSince = $null
    while ([DateTimeOffset]::UtcNow -lt $deadline) {
        $newDescendantCount = 0
        do {
            $parents = @($OwnedRootIdentity) + @($knownDescendants.Values)
            $newDescendantCount = 0
            try {
                foreach ($descendant in @(Get-TestOwnedDescendantSnapshot `
                    -OwnedParents $parents `
                    -LatestOwnedStartUtc $latestOwnedStartUtc)) {
                    $descendantProcessId = [int]$descendant.process_id
                    if ($knownDescendants.ContainsKey($descendantProcessId)) {
                        $known = $knownDescendants[$descendantProcessId]
                        if ([int]$known.parent_process_id -ne [int]$descendant.parent_process_id -or
                            ([DateTime]$known.started_at_utc).Ticks -ne
                                ([DateTime]$descendant.started_at_utc).Ticks -or
                            ([DateTime]$known.creation_utc).Ticks -ne
                                ([DateTime]$descendant.creation_utc).Ticks) {
                            throw "DESKTOP_MANAGED_CONTROL_DESCENDANT_GENERATION_CHANGED:$descendantProcessId"
                        }
                        continue
                    }
                    $knownDescendants[$descendantProcessId] = $descendant
                    $newDescendantCount++
                }
                $lastDiscoveryFailure = $null
            }
            catch { $lastDiscoveryFailure = $_ }
        } while ($newDescendantCount -gt 0 -and [DateTimeOffset]::UtcNow -lt $deadline)

        foreach ($owned in @($knownDescendants.Values)) {
            if (Test-ProcessAlive ([Diagnostics.Process]$owned.process)) {
                try { ([Diagnostics.Process]$owned.process).Kill() } catch { }
            }
        }
        if (Test-ProcessAlive $OwnedRoot) { try { $OwnedRoot.Kill() } catch { } }

        $aliveDescendants = @($knownDescendants.Values | Where-Object {
            Test-ProcessAlive ([Diagnostics.Process]$_.process)
        })
        if (-not (Test-ProcessAlive $OwnedRoot) -and $aliveDescendants.Count -eq 0 -and
            $newDescendantCount -eq 0 -and $null -eq $lastDiscoveryFailure) {
            if ($null -eq $quietSince) { $quietSince = [DateTimeOffset]::UtcNow }
            elseif (([DateTimeOffset]::UtcNow - $quietSince).TotalMilliseconds -ge 500) {
                $descendantIdentities = @($knownDescendants.Values |
                    Sort-Object process_id | ForEach-Object {
                        [PSCustomObject][ordered]@{
                            process_id = [int]$_.process_id
                            parent_process_id = [int]$_.parent_process_id
                            started_at_utc_ticks = ([DateTime]$_.started_at_utc).Ticks
                            creation_utc_ticks = ([DateTime]$_.creation_utc).Ticks
                            stopped = -not (Test-ProcessAlive ([Diagnostics.Process]$_.process))
                        }
                    })
                return [PSCustomObject][ordered]@{
                    root_process_id = $rootProcessId
                    root_started_at_utc_ticks = ([DateTime]$OwnedRootIdentity.started_at_utc).Ticks
                    root_creation_utc_ticks = ([DateTime]$OwnedRootIdentity.creation_utc).Ticks
                    descendant_process_identities = $descendantIdentities
                    root_stopped = $true
                    descendants_stopped = $true
                }
            }
        }
        else { $quietSince = $null }
        Start-Sleep -Milliseconds 50
    }

    if ($null -ne $lastDiscoveryFailure) {
        throw ('DESKTOP_MANAGED_CONTROL_DESCENDANT_DISCOVERY_FAILED:' +
            $lastDiscoveryFailure.Exception.Message)
    }
    $remainingProcessIds = @(
        if (Test-ProcessAlive $OwnedRoot) { $rootProcessId }
        $knownDescendants.Values | Where-Object {
            Test-ProcessAlive ([Diagnostics.Process]$_.process)
        } | ForEach-Object { [int]$_.process_id })
    throw ('DESKTOP_MANAGED_CONTROL_OWNED_PROCESS_TREE_STOP_TIMEOUT:' +
        (($remainingProcessIds | Sort-Object) -join ','))
}

function Test-ProcessIdentityAlive {
    param([int]$ProcessId, [Int64]$StartedAtUtcTicks)
    try {
        $process = [Diagnostics.Process]::GetProcessById($ProcessId)
        if ($process.StartTime.ToUniversalTime().Ticks -ne $StartedAtUtcTicks) {
            return $false
        }
        return Test-ProcessAlive $process
    }
    catch { return $false }
}

function Close-TestDesktop {
    param([AllowNull()][Diagnostics.Process]$Process)
    if (-not (Test-ProcessAlive $Process)) { return }
    try {
        $Process.CloseMainWindow() | Out-Null
        $Process.WaitForExit(10000) | Out-Null
    }
    catch {
        # The exact test-owned desktop is stopped by the bounded fallback below.
    }
    if (Test-ProcessAlive $Process) {
        Stop-TestOwnedProcess $Process
    }
}

function Test-ControlPortReachable {
    $client = [Net.Sockets.TcpClient]::new([Net.Sockets.AddressFamily]::InterNetwork)
    try {
        $task = $client.ConnectAsync('127.0.0.1', 4317)
        if (-not $task.Wait(500)) { return $false }
        return $client.Connected
    }
    catch { return $false }
    finally { $client.Dispose() }
}

function Wait-ControlPort {
    param([bool]$Reachable, [int]$Timeout)
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($Timeout)
    do {
        if ((Test-ControlPortReachable) -eq $Reachable) { return }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw "DESKTOP_MANAGED_CONTROL_PORT_STATE_TIMEOUT:$Reachable"
}

function Wait-TextFile {
    param([string]$LiteralPath, [Diagnostics.Process]$Owner, [int]$Timeout)
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($Timeout)
    do {
        if (Test-Path -LiteralPath $LiteralPath -PathType Leaf) {
            return ([string](Get-Content -LiteralPath $LiteralPath -Raw)).Trim()
        }
        if (-not (Test-ProcessAlive $Owner)) {
            throw 'DESKTOP_MANAGED_CONTROL_FIXTURE_EXITED'
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw 'DESKTOP_MANAGED_CONTROL_FIXTURE_READY_TIMEOUT'
}

function Wait-CompatibleRuntime {
    param([string]$ExpectedVersion, [int]$Timeout)
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($Timeout)
    do {
        try {
            $surface = Invoke-RestMethod -Uri 'http://127.0.0.1:4317/api/runtime' -TimeoutSec 2
            if (
                [string]$surface.schema_version -ceq 'lattice.control.runtime-surface.v2' -and
                [string]$surface.identity.schema_version -ceq 'lattice.control.runtime-identity.v1' -and
                [string]$surface.identity.product -ceq 'LATTICE_CONTROL' -and
                [string]$surface.identity.version -ceq $ExpectedVersion -and
                [string]$surface.data_scope.schema_version -ceq 'lattice.control.data-scope.v1' -and
                [string]$surface.data_scope.store -ceq 'CONTROL_SQLITE' -and
                [string]$surface.data_scope.digest -cmatch '^[a-f0-9]{64}$' -and
                $surface.reconciliation_required -eq $false -and
                [string]$surface.health -ceq 'HEALTHY'
            ) {
                return $surface
            }
        }
        catch {
            # Startup and reconnect have a bounded retry window.
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw 'DESKTOP_MANAGED_CONTROL_RUNTIME_TIMEOUT'
}

function Get-AutomationElement {
    param([Diagnostics.Process]$Desktop, [string]$AutomationId)
    $Desktop.Refresh()
    if ($Desktop.HasExited -or $Desktop.MainWindowHandle -eq 0) { return $null }
    $window = [Windows.Automation.AutomationElement]::FromHandle(
        [IntPtr]$Desktop.MainWindowHandle)
    $condition = [Windows.Automation.PropertyCondition]::new(
        [Windows.Automation.AutomationElement]::AutomationIdProperty,
        $AutomationId)
    return $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}

function Wait-AutomationItemStatus {
    param(
        [Diagnostics.Process]$Desktop,
        [string]$AutomationId,
        [string]$Expected,
        [int]$Timeout
    )
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($Timeout)
    do {
        if (-not (Test-ProcessAlive $Desktop)) {
            throw "DESKTOP_MANAGED_CONTROL_DESKTOP_EXITED:$Expected"
        }
        $element = Get-AutomationElement $Desktop $AutomationId
        if ($null -ne $element) {
            $observed = [string]$element.GetCurrentPropertyValue(
                [Windows.Automation.AutomationElement]::ItemStatusProperty)
            if ($observed -ceq $Expected) { return $observed }
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw "DESKTOP_MANAGED_CONTROL_UI_STATUS_TIMEOUT:$AutomationId`:$Expected"
}

function Start-TestDesktop {
    param(
        [string]$Executable,
        [string]$WorkingDirectory,
        [string]$LocalData,
        [string]$ExpectedExecutableSha256,
        [string]$ExpectedSourceCommit,
        [string]$TestScript = '',
        [AllowNull()][hashtable]$AdditionalEnvironment = $null,
        [string]$IdentityReadyPath = '',
        [string]$FailureCleanupRoot = '',
        [string]$IdentityCleanupEvidencePath = '',
        [int]$ReadyTimeout = 10
    )
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Executable
    $start.WorkingDirectory = $WorkingDirectory
    $start.UseShellExecute = $false
    $start.EnvironmentVariables['LOCALAPPDATA'] = $LocalData
    $start.EnvironmentVariables['WEBVIEW2_USER_DATA_FOLDER'] = Join-Path $LocalData 'webview2'
    if (-not [string]::IsNullOrWhiteSpace($TestScript)) {
        if ($TestScript.IndexOf('"') -ge 0 -or $TestScript.IndexOf([char]0) -ge 0) {
            throw 'DESKTOP_MANAGED_CONTROL_TEST_SCRIPT_PATH_INVALID'
        }
        $start.Arguments = '"' + $TestScript + '"'
    }
    if ($null -ne $AdditionalEnvironment) {
        foreach ($key in $AdditionalEnvironment.Keys) {
            if ([string]$key -ieq 'LOCALAPPDATA' -or
                [string]$key -ieq 'WEBVIEW2_USER_DATA_FOLDER') {
                throw 'DESKTOP_MANAGED_CONTROL_TEST_ENVIRONMENT_RESERVED'
            }
            $start.EnvironmentVariables[[string]$key] = [string]$AdditionalEnvironment[$key]
        }
    }
    $process = [Diagnostics.Process]::Start($start)
    if ($null -eq $process) { throw 'DESKTOP_MANAGED_CONTROL_DESKTOP_START_FAILED' }
    $ownedRootIdentity = $null
    try {
        $ownedRootIdentity = Get-TestBoundProcessIdentity -Process $process
        if (-not [string]::IsNullOrWhiteSpace($IdentityReadyPath)) {
            [void](Wait-TextFile $IdentityReadyPath $process $ReadyTimeout)
        }
        [void](Assert-CandidateProcessIdentity `
            -DesktopProcess $process `
            -ExpectedExecutablePath $Executable `
            -ExpectedExecutableSha256 $ExpectedExecutableSha256 `
            -ExpectedSourceCommit $ExpectedSourceCommit)
    }
    catch {
        $identityFailure = $_
        $cleanupErrors = [Collections.Generic.List[string]]::new()
        $treeCleanup = $null
        try {
            $treeCleanup = Stop-TestOwnedProcessTree `
                -OwnedRoot $process `
                -OwnedRootIdentity $ownedRootIdentity
        }
        catch { [void]$cleanupErrors.Add($_.Exception.Message) }
        if (-not [string]::IsNullOrWhiteSpace($FailureCleanupRoot)) {
            try { Remove-TestRoot $FailureCleanupRoot }
            catch { [void]$cleanupErrors.Add($_.Exception.Message) }
        }
        if ($cleanupErrors.Count -eq 0 -and
            -not [string]::IsNullOrWhiteSpace($IdentityCleanupEvidencePath)) {
            try {
                $evidenceFull = [IO.Path]::GetFullPath($IdentityCleanupEvidencePath)
                $tempPrefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
                    [IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
                if (-not $evidenceFull.StartsWith(
                    $tempPrefix,
                    [StringComparison]::OrdinalIgnoreCase)) {
                    throw 'DESKTOP_MANAGED_CONTROL_IDENTITY_EVIDENCE_PATH_INVALID'
                }
                [IO.File]::WriteAllText(
                    $evidenceFull,
                    ($treeCleanup | ConvertTo-Json -Depth 5),
                    [Text.UTF8Encoding]::new($false))
            }
            catch { [void]$cleanupErrors.Add($_.Exception.Message) }
        }
        if ($cleanupErrors.Count -gt 0) {
            throw ('DESKTOP_MANAGED_CONTROL_IDENTITY_FAILURE_CLEANUP_FAILED:' +
                ($cleanupErrors -join '|') +
                ':IDENTITY_FAILURE:' + $identityFailure.Exception.Message)
        }
        throw $identityFailure
    }
    return $process
}

function Start-ControlProcess {
    param(
        [string]$Executable,
        [string]$Script,
        [string]$WorkingDirectory,
        [hashtable]$Environment
    )
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Executable
    $start.WorkingDirectory = $WorkingDirectory
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    if ($Script.IndexOf('"') -ge 0 -or $Script.IndexOf([char]0) -ge 0) {
        throw 'DESKTOP_MANAGED_CONTROL_SCRIPT_PATH_INVALID'
    }
    $start.Arguments = '"' + $Script + '"'
    foreach ($key in $Environment.Keys) {
        $start.EnvironmentVariables[[string]$key] = [string]$Environment[$key]
    }
    $process = [Diagnostics.Process]::Start($start)
    if ($null -eq $process) { throw 'DESKTOP_MANAGED_CONTROL_FIXTURE_START_FAILED' }
    return $process
}

function Get-DesktopControlChild {
    param([Diagnostics.Process]$Desktop, [string]$ExpectedNode, [int]$Timeout)
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($Timeout)
    $expectedNodeFull = [IO.Path]::GetFullPath($ExpectedNode)
    do {
        $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($Desktop.Id)" |
            Where-Object {
                $_.Name -ieq 'node.exe' -and
                -not [string]::IsNullOrWhiteSpace([string]$_.ExecutablePath) -and
                [IO.Path]::GetFullPath([string]$_.ExecutablePath) -ieq $expectedNodeFull
            })
        if ($children.Count -gt 1) {
            throw 'DESKTOP_MANAGED_CONTROL_MULTIPLE_OWNED_CONTROLS'
        }
        if ($children.Count -eq 1) {
            return [Diagnostics.Process]::GetProcessById([int]$children[0].ProcessId)
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    return $null
}

function Remove-TestRoot {
    param([string]$LiteralPath)
    $full = [IO.Path]::GetFullPath($LiteralPath)
    $tempPrefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
        [IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    if (-not $full.StartsWith($tempPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        -not [IO.Path]::GetFileName($full).StartsWith(
            'lattice-managed-control-', [StringComparison]::Ordinal)) {
        throw 'DESKTOP_MANAGED_CONTROL_TEMP_ROOT_INVALID'
    }

    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(10)
    $lastFailure = $null
    do {
        if (-not (Test-Path -LiteralPath $full)) { return }
        try {
            Remove-Item -LiteralPath $full -Recurse -Force
            if (-not (Test-Path -LiteralPath $full)) { return }
        }
        catch { $lastFailure = $_ }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    if ($null -ne $lastFailure) { throw $lastFailure }
    throw 'DESKTOP_MANAGED_CONTROL_TEMP_ROOT_REMOVE_TIMEOUT'
}

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$headSha = ([string](& git -C $repositoryRoot rev-parse HEAD)).Trim()
if ($LASTEXITCODE -ne 0 -or $headSha -notmatch '^[0-9a-f]{40}$') {
    throw 'DESKTOP_MANAGED_CONTROL_GIT_HEAD_UNAVAILABLE'
}
if ([string]::IsNullOrWhiteSpace($CandidateArchive)) {
    $candidateRoot = Join-Path (
        [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
    ) 'LATTICE\candidates'
    $CandidateArchive = Join-Path $candidateRoot (
        'lattice-control-desktop-win-x64-' + $headSha.Substring(0, 12) + '.zip')
}
$candidateArchiveFull = [IO.Path]::GetFullPath($CandidateArchive)
if (-not (Test-Path -LiteralPath $candidateArchiveFull -PathType Leaf)) {
    throw 'DESKTOP_MANAGED_CONTROL_ARCHIVE_MISSING'
}
if (Test-ControlPortReachable) {
    throw 'DESKTOP_MANAGED_CONTROL_PREEXISTING_4317_LISTENER'
}

$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) (
    'lattice-managed-control-' + [Guid]::NewGuid().ToString('N'))
$candidateDirectory = Join-Path $temporaryRoot 'candidate'
$managedData = Join-Path $temporaryRoot 'managed-data'
$reuseData = Join-Path $temporaryRoot 'reuse-data'
$externalData = Join-Path $temporaryRoot 'external-data'
$unknownData = Join-Path $temporaryRoot 'unknown-data'
$foreignReady = Join-Path $temporaryRoot 'foreign-ready.txt'
$unknownReady = Join-Path $temporaryRoot 'unknown-ready.txt'
$mismatchReady = Join-Path $temporaryRoot 'mismatch-ready.json'
$mismatchCleanupEvidencePath = Join-Path $temporaryRoot 'mismatch-cleanup.json'
$identityExternalReady = Join-Path $temporaryRoot 'identity-external-ready.json'
$mismatchRoot = Join-Path ([IO.Path]::GetTempPath()) (
    'lattice-managed-control-mismatch-' + [Guid]::NewGuid().ToString('N'))
$mismatchStore = Join-Path $mismatchRoot 'LATTICE\control\mismatch-store.txt'
$desktop = $null
$managedControl = $null
$externalControl = $null
$crossScopeControl = $null
$foreignControl = $null
$unknownControl = $null
$identityExternalFixture = $null
$result = $null
$initialDesktopProcessIdentity = $null
$wrongRevisionRejected = $false
$mismatchDesktopStopped = $false
$mismatchDescendantsStopped = $false
$mismatchPortReleased = $false
$mismatchStoreAndTempRemoved = $false
$externalFixtureSurvivedIdentityMismatch = $false
$primaryFailure = $null
$cleanupFailure = $null

try {
    [IO.Directory]::CreateDirectory($temporaryRoot) | Out-Null
    Expand-Archive -LiteralPath $candidateArchiveFull -DestinationPath $candidateDirectory
    $manifestPath = Join-Path $candidateDirectory 'candidate-manifest.json'
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ([string]$manifest.schema_version -cne 'lattice.control.desktop-portable-candidate.v2' -or
        [string]$manifest.source_commit -cne $headSha) {
        throw 'DESKTOP_MANAGED_CONTROL_MANIFEST_MISMATCH'
    }
    $expectedVersion = [string]$manifest.control_runtime.version
    $candidateExecutableSha256 = [string]$manifest.executable_sha256
    if ($candidateExecutableSha256 -cnotmatch '^[0-9a-f]{64}$') {
        throw 'DESKTOP_MANAGED_CONTROL_EXECUTABLE_HASH_INVALID'
    }
    $candidateExecutable = Join-Path $candidateDirectory 'LATTICE.exe'
    $runtimeRoot = Join-Path $candidateDirectory 'control-runtime'
    $runtimeNode = Join-Path $runtimeRoot 'node.exe'
    $runtimeServer = Join-Path $runtimeRoot 'apps\lattice-control\src\server.mjs'
    foreach ($required in @($candidateExecutable, $runtimeNode, $runtimeServer)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
            throw 'DESKTOP_MANAGED_CONTROL_RUNTIME_FILE_MISSING'
        }
    }

    $nodePath = [IO.Path]::GetFullPath(
        (Get-Command node.exe -CommandType Application -ErrorAction Stop).Source)
    $mismatchFixture = Join-Path $repositoryRoot (
        'apps\lattice-control\test\fixtures\desktop-mismatched-candidate.mjs')
    if (-not (Test-Path -LiteralPath $mismatchFixture -PathType Leaf)) {
        throw 'DESKTOP_MANAGED_CONTROL_MISMATCH_FIXTURE_MISSING'
    }

    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes

    # A real mismatched executable is rejected only after its owned child has
    # created a store and bound 4317. Identity failure must tear down that exact
    # process tree while leaving an unrelated sibling fixture alive.
    $identityExternalFixture = Start-ControlProcess $nodePath $mismatchFixture $repositoryRoot @{
        'LATTICE_DESKTOP_MISMATCH_ROLE' = 'external'
        'LATTICE_DESKTOP_MISMATCH_READY' = $identityExternalReady
    }
    $identityExternalEvidence = (Wait-TextFile `
        $identityExternalReady `
        $identityExternalFixture `
        $TimeoutSeconds) | ConvertFrom-Json
    if ([int]$identityExternalEvidence.pid -ne $identityExternalFixture.Id -or
        [int]$identityExternalEvidence.port -lt 1) {
        throw 'DESKTOP_MANAGED_CONTROL_EXTERNAL_FIXTURE_IDENTITY_INVALID'
    }

    try {
        [void](Start-TestDesktop `
            -Executable $nodePath `
            -WorkingDirectory $repositoryRoot `
            -LocalData $mismatchRoot `
            -ExpectedExecutableSha256 (Get-Sha256Hex -LiteralPath $nodePath) `
            -ExpectedSourceCommit $headSha `
            -TestScript $mismatchFixture `
            -AdditionalEnvironment @{
                'LATTICE_DESKTOP_MISMATCH_ROLE' = 'root'
                'LATTICE_DESKTOP_MISMATCH_READY' = $mismatchReady
                'LATTICE_DESKTOP_MISMATCH_STORE' = $mismatchStore
            } `
            -IdentityReadyPath $mismatchReady `
            -FailureCleanupRoot $mismatchRoot `
            -IdentityCleanupEvidencePath $mismatchCleanupEvidencePath `
            -ReadyTimeout $TimeoutSeconds)
    }
    catch {
        if ($_.Exception.Message -cne 'DESKTOP_MANAGED_CONTROL_PROCESS_REVISION_MISMATCH') {
            throw
        }
        $wrongRevisionRejected = $true
    }
    if (-not $wrongRevisionRejected) {
        throw 'DESKTOP_MANAGED_CONTROL_WRONG_REVISION_ACCEPTED'
    }
    $mismatchEvidence = ([string](Get-Content -LiteralPath $mismatchReady -Raw)).Trim() |
        ConvertFrom-Json
    if ([int]$mismatchEvidence.port -ne 4317 -or
        [int]$mismatchEvidence.root_pid -lt 1 -or
        [int]$mismatchEvidence.child_pid -lt 1) {
        throw 'DESKTOP_MANAGED_CONTROL_MISMATCH_FIXTURE_EVIDENCE_INVALID'
    }
    $mismatchCleanupEvidence = Get-Content `
        -LiteralPath $mismatchCleanupEvidencePath `
        -Raw | ConvertFrom-Json
    $mismatchChildCleanupIdentity = @(
        $mismatchCleanupEvidence.descendant_process_identities | Where-Object {
            [int]$_.process_id -eq [int]$mismatchEvidence.child_pid -and
            [int]$_.parent_process_id -eq [int]$mismatchEvidence.root_pid
        })
    if ([int]$mismatchCleanupEvidence.root_process_id -ne [int]$mismatchEvidence.root_pid -or
        $mismatchChildCleanupIdentity.Count -ne 1) {
        throw 'DESKTOP_MANAGED_CONTROL_MISMATCH_CLEANUP_IDENTITY_INVALID'
    }
    $mismatchDesktopStopped = $mismatchCleanupEvidence.root_stopped -eq $true -and
        -not (Test-ProcessIdentityAlive `
            -ProcessId ([int]$mismatchCleanupEvidence.root_process_id) `
            -StartedAtUtcTicks ([Int64]$mismatchCleanupEvidence.root_started_at_utc_ticks))
    $mismatchDescendantsStopped = $mismatchCleanupEvidence.descendants_stopped -eq $true -and
        $mismatchChildCleanupIdentity[0].stopped -eq $true -and
        -not (Test-ProcessIdentityAlive `
            -ProcessId ([int]$mismatchChildCleanupIdentity[0].process_id) `
            -StartedAtUtcTicks ([Int64]$mismatchChildCleanupIdentity[0].started_at_utc_ticks))
    Wait-ControlPort $false $TimeoutSeconds
    $mismatchPortReleased = -not (Test-ControlPortReachable)
    $mismatchStoreAndTempRemoved = -not (Test-Path -LiteralPath $mismatchStore) -and
        -not (Test-Path -LiteralPath $mismatchRoot)
    $externalFixtureSurvivedIdentityMismatch = Test-ProcessAlive $identityExternalFixture
    if (-not $mismatchDesktopStopped -or
        -not $mismatchDescendantsStopped -or
        -not $mismatchPortReleased -or
        -not $mismatchStoreAndTempRemoved -or
        -not $externalFixtureSurvivedIdentityMismatch) {
        throw 'DESKTOP_MANAGED_CONTROL_MISMATCH_TEARDOWN_INCOMPLETE'
    }
    Stop-TestOwnedProcess $identityExternalFixture
    $identityExternalFixture = $null

    # No listener: LATTICE must start one owned, compatible Control.
    $desktop = Start-TestDesktop `
        $candidateExecutable `
        $candidateDirectory `
        $managedData `
        $candidateExecutableSha256 `
        $headSha
    $initialDesktopProcessIdentity = Assert-CandidateProcessIdentity `
        -DesktopProcess $desktop `
        -ExpectedExecutablePath $candidateExecutable `
        -ExpectedExecutableSha256 $candidateExecutableSha256 `
        -ExpectedSourceCommit $headSha
    Wait-CompatibleRuntime $expectedVersion $TimeoutSeconds | Out-Null
    Wait-AutomationItemStatus $desktop 'LatticeConnectionStatus' 'connected' $TimeoutSeconds | Out-Null
    $managedControl = Get-DesktopControlChild $desktop $runtimeNode $TimeoutSeconds
    if ($null -eq $managedControl) { throw 'DESKTOP_MANAGED_CONTROL_OWNED_CHILD_MISSING' }
    $initialOwnedPid = $managedControl.Id

    # Interruption: the UI must diagnose STOPPED, then start a fresh owned PID.
    Stop-TestOwnedProcess $managedControl
    Wait-AutomationItemStatus $desktop 'LatticeRuntimeHealth' 'STOPPED' $TimeoutSeconds | Out-Null
    Wait-CompatibleRuntime $expectedVersion $TimeoutSeconds | Out-Null
    Wait-AutomationItemStatus $desktop 'LatticeConnectionStatus' 'connected' $TimeoutSeconds | Out-Null
    $managedControl = Get-DesktopControlChild $desktop $runtimeNode $TimeoutSeconds
    if ($null -eq $managedControl -or $managedControl.Id -eq $initialOwnedPid) {
        throw 'DESKTOP_MANAGED_CONTROL_REPLACEMENT_PID_INVALID'
    }
    $replacementOwnedPid = $managedControl.Id
    Close-TestDesktop $desktop
    $desktop = $null
    if (Test-ProcessAlive $managedControl) {
        throw 'DESKTOP_MANAGED_CONTROL_OWNED_CHILD_SURVIVED_DESKTOP'
    }
    Wait-ControlPort $false $TimeoutSeconds
    $managedControl = $null

    # Compatible listener: reuse it, launch no child, and leave it alive on close.
    $reuseDatabase = Join-Path $reuseData 'LATTICE\control\lattice-control.db'
    $externalControl = Start-ControlProcess $runtimeNode $runtimeServer $repositoryRoot @{
        'LATTICE_CONTROL_PORT' = '4317'
        'LOCALAPPDATA' = $reuseData
        'LATTICE_CONTROL_DATABASE_PATH' = $reuseDatabase
    }
    $reuseSurface = Wait-CompatibleRuntime $expectedVersion $TimeoutSeconds
    $reuseScopeDigest = [string]$reuseSurface.data_scope.digest
    $desktop = Start-TestDesktop $candidateExecutable $candidateDirectory $reuseData $candidateExecutableSha256 $headSha
    Wait-AutomationItemStatus $desktop 'LatticeConnectionStatus' 'connected' $TimeoutSeconds | Out-Null
    if ($null -ne (Get-DesktopControlChild $desktop $runtimeNode 2)) {
        throw 'DESKTOP_MANAGED_CONTROL_REUSE_STARTED_CHILD'
    }
    Close-TestDesktop $desktop
    $desktop = $null
    if (-not (Test-ProcessAlive $externalControl)) {
        throw 'DESKTOP_MANAGED_CONTROL_REUSED_PROCESS_STOPPED'
    }
    Wait-CompatibleRuntime $expectedVersion 5 | Out-Null
    $reusedPid = $externalControl.Id
    Stop-TestOwnedProcess $externalControl
    $externalControl = $null
    Wait-ControlPort $false $TimeoutSeconds

    # Same version but a different SQLite scope on fixed 4317: fail closed,
    # launch no child, and leave the different-scope listener alive.
    $crossScopeDatabase = Join-Path $externalData 'LATTICE\control\lattice-control.db'
    $crossScopeControl = Start-ControlProcess $runtimeNode $runtimeServer $repositoryRoot @{
        'LATTICE_CONTROL_PORT' = '4317'
        'LOCALAPPDATA' = $externalData
        'LATTICE_CONTROL_DATABASE_PATH' = $crossScopeDatabase
    }
    $crossScopeSurface = Wait-CompatibleRuntime $expectedVersion $TimeoutSeconds
    $crossScopeDigest = [string]$crossScopeSurface.data_scope.digest
    if ($crossScopeDigest -ceq $reuseScopeDigest) {
        throw 'DESKTOP_MANAGED_CONTROL_CROSS_SCOPE_DIGEST_COLLISION'
    }
    $desktop = Start-TestDesktop $candidateExecutable $candidateDirectory $reuseData $candidateExecutableSha256 $headSha
    Wait-AutomationItemStatus $desktop 'LatticeRuntimeHealth' 'INCOMPATIBLE' $TimeoutSeconds | Out-Null
    if ($null -ne (Get-DesktopControlChild $desktop $runtimeNode 2)) {
        throw 'DESKTOP_MANAGED_CONTROL_CROSS_SCOPE_STARTED_CHILD'
    }
    Close-TestDesktop $desktop
    $desktop = $null
    if (-not (Test-ProcessAlive $crossScopeControl)) {
        throw 'DESKTOP_MANAGED_CONTROL_CROSS_SCOPE_PROCESS_STOPPED'
    }
    $crossScopePid = $crossScopeControl.Id
    Stop-TestOwnedProcess $crossScopeControl
    $crossScopeControl = $null
    Wait-ControlPort $false $TimeoutSeconds

    $foreignFixture = Join-Path $repositoryRoot (
        'apps\lattice-control\test\fixtures\desktop-incompatible-control.mjs')

    # Unknown/malformed runtime on fixed 4317: the packaged candidate must fail
    # closed, launch no bundled child, and never stop the foreign listener.
    $unknownDatabase = Join-Path $unknownData 'LATTICE\control\lattice-control.db'
    $unknownStoreDirectory = Split-Path -Parent $unknownDatabase
    if (Test-Path -LiteralPath $unknownStoreDirectory) {
        throw 'DESKTOP_MANAGED_CONTROL_UNKNOWN_STORE_PREEXISTING'
    }
    $unknownControl = Start-ControlProcess $nodePath $foreignFixture $repositoryRoot @{
        'LATTICE_DESKTOP_INCOMPATIBLE_PORT' = '4317'
        'LATTICE_DESKTOP_INCOMPATIBLE_READY' = $unknownReady
        'LATTICE_DESKTOP_INCOMPATIBLE_MODE' = 'malformed'
    }
    if ((Wait-TextFile $unknownReady $unknownControl $TimeoutSeconds) -cne '4317') {
        throw 'DESKTOP_MANAGED_CONTROL_UNKNOWN_PORT_INVALID'
    }
    $unknownSurface = Invoke-WebRequest `
        -Uri 'http://127.0.0.1:4317/api/runtime' `
        -TimeoutSec 2 `
        -UseBasicParsing
    if ($unknownSurface.StatusCode -ne 200 -or
        [string]$unknownSurface.Content -cne 'not-a-lattice-runtime') {
        throw 'DESKTOP_MANAGED_CONTROL_UNKNOWN_SURFACE_NOT_MALFORMED'
    }
    $desktop = Start-TestDesktop $candidateExecutable $candidateDirectory $unknownData $candidateExecutableSha256 $headSha
    Wait-AutomationItemStatus $desktop 'LatticeRuntimeHealth' 'INCOMPATIBLE' $TimeoutSeconds | Out-Null
    if (Test-Path -LiteralPath $unknownStoreDirectory) {
        throw 'DESKTOP_MANAGED_CONTROL_UNKNOWN_BUNDLED_STORE_CREATED'
    }
    if ($null -ne (Get-DesktopControlChild $desktop $runtimeNode 2)) {
        throw 'DESKTOP_MANAGED_CONTROL_UNKNOWN_STARTED_CHILD'
    }
    Close-TestDesktop $desktop
    $desktop = $null
    if (-not (Test-ProcessAlive $unknownControl)) {
        throw 'DESKTOP_MANAGED_CONTROL_UNKNOWN_PROCESS_STOPPED'
    }
    $unknownAfterClose = Invoke-WebRequest `
        -Uri 'http://127.0.0.1:4317/api/runtime' `
        -TimeoutSec 2 `
        -UseBasicParsing
    if ($unknownAfterClose.StatusCode -ne 200 -or
        [string]$unknownAfterClose.Content -cne 'not-a-lattice-runtime') {
        throw 'DESKTOP_MANAGED_CONTROL_UNKNOWN_LISTENER_NOT_SERVING_AFTER_CLOSE'
    }
    if (Test-Path -LiteralPath $unknownStoreDirectory) {
        throw 'DESKTOP_MANAGED_CONTROL_UNKNOWN_BUNDLED_STORE_CREATED_AFTER_CLOSE'
    }
    $unknownPid = $unknownControl.Id
    Stop-TestOwnedProcess $unknownControl
    $unknownControl = $null
    Wait-ControlPort $false $TimeoutSeconds

    # Wrong version on 4317: fail closed, launch no child, and never stop it.
    $foreignControl = Start-ControlProcess $nodePath $foreignFixture $repositoryRoot @{
        'LATTICE_DESKTOP_INCOMPATIBLE_PORT' = '4317'
        'LATTICE_DESKTOP_INCOMPATIBLE_READY' = $foreignReady
        'LATTICE_CONTROL_DATABASE_PATH' = $reuseDatabase
    }
    if ((Wait-TextFile $foreignReady $foreignControl $TimeoutSeconds) -cne '4317') {
        throw 'DESKTOP_MANAGED_CONTROL_FOREIGN_PORT_INVALID'
    }
    $desktop = Start-TestDesktop $candidateExecutable $candidateDirectory $reuseData $candidateExecutableSha256 $headSha
    Wait-AutomationItemStatus $desktop 'LatticeRuntimeHealth' 'INCOMPATIBLE' $TimeoutSeconds | Out-Null
    if ($null -ne (Get-DesktopControlChild $desktop $runtimeNode 2)) {
        throw 'DESKTOP_MANAGED_CONTROL_INCOMPATIBLE_STARTED_CHILD'
    }
    Close-TestDesktop $desktop
    $desktop = $null
    if (-not (Test-ProcessAlive $foreignControl)) {
        throw 'DESKTOP_MANAGED_CONTROL_FOREIGN_PROCESS_STOPPED'
    }
    $foreignPid = $foreignControl.Id
    Stop-TestOwnedProcess $foreignControl
    $foreignControl = $null
    Wait-ControlPort $false $TimeoutSeconds

    $result = [ordered]@{
        result = 'PASS'
        source_commit = $headSha
        candidate_archive = $candidateArchiveFull
        desktop_process_identity = $initialDesktopProcessIdentity
        wrong_revision_rejected = $wrongRevisionRejected
        mismatch_desktop_stopped = $mismatchDesktopStopped
        mismatch_descendants_stopped = $mismatchDescendantsStopped
        mismatch_port_released = $mismatchPortReleased
        mismatch_store_and_temp_removed = $mismatchStoreAndTempRemoved
        external_fixture_survived_identity_mismatch = $externalFixtureSurvivedIdentityMismatch
        mismatch_root_pid = [int]$mismatchEvidence.root_pid
        mismatch_child_pid = [int]$mismatchEvidence.child_pid
        production_control_port = 4317
        no_listener_started_owned_control = $true
        initial_owned_pid = $initialOwnedPid
        interruption_observed_status = 'STOPPED'
        reconnect_new_pid = $replacementOwnedPid
        owned_control_stopped_on_close = $true
        compatible_control_reused = $true
        reused_control_pid = $reusedPid
        reused_control_survived_close = $true
        same_scope_digest = $reuseScopeDigest
        cross_scope_status = 'INCOMPATIBLE'
        cross_scope_digest = $crossScopeDigest
        cross_scope_process_pid = $crossScopePid
        cross_scope_process_survived_close = $true
        unknown_runtime_status = 'INCOMPATIBLE'
        unknown_runtime_bundled_child_started = $false
        unknown_runtime_bundled_store_artifacts_created = $false
        unknown_runtime_process_pid = $unknownPid
        unknown_runtime_process_survived_close = $true
        unknown_runtime_surface_served_after_close = $true
        incompatible_status = 'INCOMPATIBLE'
        incompatible_process_pid = $foreignPid
        incompatible_process_survived_close = $true
        final_port_reachable = Test-ControlPortReachable
    }
}
catch { $primaryFailure = $_ }
finally {
    try { Close-TestDesktop $desktop } catch { $cleanupFailure = $_ }
    foreach ($owned in @(
        $managedControl,
        $externalControl,
        $crossScopeControl,
        $unknownControl,
        $foreignControl,
        $identityExternalFixture
    )) {
        try { Stop-TestOwnedProcess $owned } catch { if ($null -eq $cleanupFailure) { $cleanupFailure = $_ } }
    }
    try { Remove-TestRoot $mismatchRoot } catch { if ($null -eq $cleanupFailure) { $cleanupFailure = $_ } }
    try { Remove-TestRoot $temporaryRoot } catch { if ($null -eq $cleanupFailure) { $cleanupFailure = $_ } }
}

if ($null -ne $primaryFailure) {
    if ($null -ne $cleanupFailure) {
        Write-Warning "DESKTOP_MANAGED_CONTROL_CLEANUP_FAILED:$($cleanupFailure.Exception.Message)"
    }
    throw $primaryFailure
}
if ($null -ne $cleanupFailure) { throw $cleanupFailure }
if ($null -eq $result -or $result.final_port_reachable) {
    throw 'DESKTOP_MANAGED_CONTROL_ACCEPTANCE_INCOMPLETE'
}
$result | ConvertTo-Json -Depth 4
