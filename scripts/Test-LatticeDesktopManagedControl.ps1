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
    param([string]$Executable, [string]$WorkingDirectory, [string]$LocalData)
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Executable
    $start.WorkingDirectory = $WorkingDirectory
    $start.UseShellExecute = $false
    $start.EnvironmentVariables['LOCALAPPDATA'] = $LocalData
    $start.EnvironmentVariables['WEBVIEW2_USER_DATA_FOLDER'] = Join-Path $LocalData 'webview2'
    $process = [Diagnostics.Process]::Start($start)
    if ($null -eq $process) { throw 'DESKTOP_MANAGED_CONTROL_DESKTOP_START_FAILED' }
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

    if (Test-Path -LiteralPath $full) {
        Remove-Item -LiteralPath $full -Recurse -Force
    }
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
$desktop = $null
$managedControl = $null
$externalControl = $null
$crossScopeControl = $null
$foreignControl = $null
$unknownControl = $null
$result = $null
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
    $candidateExecutable = Join-Path $candidateDirectory 'LATTICE.exe'
    $runtimeRoot = Join-Path $candidateDirectory 'control-runtime'
    $runtimeNode = Join-Path $runtimeRoot 'node.exe'
    $runtimeServer = Join-Path $runtimeRoot 'apps\lattice-control\src\server.mjs'
    foreach ($required in @($candidateExecutable, $runtimeNode, $runtimeServer)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
            throw 'DESKTOP_MANAGED_CONTROL_RUNTIME_FILE_MISSING'
        }
    }

    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes

    # No listener: LATTICE must start one owned, compatible Control.
    $desktop = Start-TestDesktop $candidateExecutable $candidateDirectory $managedData
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
    $desktop = Start-TestDesktop $candidateExecutable $candidateDirectory $reuseData
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
    $desktop = Start-TestDesktop $candidateExecutable $candidateDirectory $reuseData
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

    $nodePath = [IO.Path]::GetFullPath(
        (Get-Command node.exe -CommandType Application -ErrorAction Stop).Source)
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
    $desktop = Start-TestDesktop $candidateExecutable $candidateDirectory $unknownData
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
    $desktop = Start-TestDesktop $candidateExecutable $candidateDirectory $reuseData
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
        $foreignControl
    )) {
        try { Stop-TestOwnedProcess $owned } catch { if ($null -eq $cleanupFailure) { $cleanupFailure = $_ } }
    }
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
