[CmdletBinding()]
param(
    [string]$CandidateArchive = '',
    [ValidateRange(1, 3600)]
    [int]$MinimumLifetimeSeconds = 960,
    [ValidateRange(5, 120)]
    [int]$ReconnectTimeoutSeconds = 45
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Text.UTF8Encoding]::new($false)

$offlineConnectionStatus = 'LATTICE ' + [string][char]0x672A + [char]0x9023 + [char]0x7DDA
$connectedConnectionStatus = [string][char]0x672C + [char]0x6A5F + ' LATTICE ' +
    [char]0x5DF2 + [char]0x9023 + [char]0x7DDA

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
        finally {
            $algorithm.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Get-RequiredPropertyValue {
    param(
        [object]$InputObject,
        [string]$Name
    )

    $property = $InputObject.PSObject.Properties[$Name]
    if ($null -eq $property) {
        throw "DESKTOP_CANDIDATE_MANIFEST_PROPERTY_MISSING:$Name"
    }
    return $property.Value
}

function Get-ConnectionElement {
    param([Diagnostics.Process]$DesktopProcess)

    $DesktopProcess.Refresh()
    if ($DesktopProcess.HasExited -or $DesktopProcess.MainWindowHandle -eq 0) {
        return $null
    }
    $window = [Windows.Automation.AutomationElement]::FromHandle(
        [IntPtr]$DesktopProcess.MainWindowHandle)
    $condition = [Windows.Automation.PropertyCondition]::new(
        [Windows.Automation.AutomationElement]::AutomationIdProperty,
        'LatticeConnectionStatus')
    return $window.FindFirst(
        [Windows.Automation.TreeScope]::Descendants,
        $condition)
}

function Get-ConnectionStatus {
    param([Diagnostics.Process]$DesktopProcess)

    $element = Get-ConnectionElement -DesktopProcess $DesktopProcess
    if ($null -eq $element) {
        return $null
    }
    return [string]$element.GetCurrentPropertyValue(
        [Windows.Automation.AutomationElement]::NameProperty)
}

function Get-ConnectionItemStatus {
    param([Diagnostics.Process]$DesktopProcess)

    $element = Get-ConnectionElement -DesktopProcess $DesktopProcess
    if ($null -eq $element) {
        return $null
    }
    return [string]$element.GetCurrentPropertyValue(
        [Windows.Automation.AutomationElement]::ItemStatusProperty)
}

function Wait-ConnectionStatus {
    param(
        [Diagnostics.Process]$DesktopProcess,
        [string]$Expected,
        [int]$TimeoutSeconds
    )

    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if ($DesktopProcess.HasExited) {
            throw "DESKTOP_CANDIDATE_EXITED_WHILE_WAITING_FOR:$Expected"
        }
        $observed = Get-ConnectionStatus -DesktopProcess $DesktopProcess
        if ($observed -eq $Expected) {
            return $observed
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw "DESKTOP_CANDIDATE_STATUS_TIMEOUT:$Expected"
}

function Wait-ConnectionItemStatus {
    param(
        [Diagnostics.Process]$DesktopProcess,
        [string]$Expected,
        [int]$TimeoutSeconds
    )

    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if ($DesktopProcess.HasExited) {
            throw "DESKTOP_CANDIDATE_EXITED_WHILE_WAITING_FOR_ITEM_STATUS:$Expected"
        }
        $observed = Get-ConnectionItemStatus -DesktopProcess $DesktopProcess
        if ($observed -eq $Expected) {
            return $observed
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw "DESKTOP_CANDIDATE_ITEM_STATUS_TIMEOUT:$Expected"
}

function Wait-ProcessTextFile {
    param(
        [string]$Path,
        [Diagnostics.Process]$OwnerProcess,
        [int]$TimeoutSeconds,
        [string]$FailureCode
    )

    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            $value = ([string](Get-Content -LiteralPath $Path -Raw)).Trim()
            if (-not [string]::IsNullOrWhiteSpace($value)) {
                return $value
            }
        }
        if ($OwnerProcess.HasExited) {
            throw "$FailureCode`:PROCESS_EXIT_$($OwnerProcess.ExitCode)"
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw "$FailureCode`:TIMEOUT"
}

function ConvertTo-LoopbackPort {
    param(
        [string]$Value,
        [string]$FailureCode
    )

    $port = 0
    if (-not [int]::TryParse($Value, [ref]$port) -or $port -lt 1 -or $port -gt 65535 -or $port -eq 4317) {
        throw $FailureCode
    }
    return $port
}

function Stop-OwnedProcess {
    param([AllowNull()][Diagnostics.Process]$OwnedProcess)

    if ($null -eq $OwnedProcess -or $OwnedProcess.HasExited) {
        return
    }
    try {
        $OwnedProcess.Kill()
    }
    catch {
        $OwnedProcess.Refresh()
        if ($OwnedProcess.HasExited) {
            return
        }
        throw
    }
    if (-not $OwnedProcess.WaitForExit(10000)) {
        throw "DESKTOP_CANDIDATE_OWNED_PROCESS_STOP_TIMEOUT:$($OwnedProcess.Id)"
    }
    $OwnedProcess.Refresh()
    if (-not $OwnedProcess.HasExited) {
        throw "DESKTOP_CANDIDATE_OWNED_PROCESS_STOP_TIMEOUT:$($OwnedProcess.Id)"
    }
}

function Remove-TemporaryRootWithRetry {
    param(
        [string]$LiteralPath,
        [ValidateRange(1, 120)]
        [int]$TimeoutSeconds = 30
    )

    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastFailure = $null
    do {
        try {
            if (-not (Test-Path -LiteralPath $LiteralPath)) {
                return
            }
            Remove-Item -LiteralPath $LiteralPath -Recurse -Force -ErrorAction Stop
            if (-not (Test-Path -LiteralPath $LiteralPath)) {
                return
            }
        }
        catch {
            $lastFailure = $_
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTimeOffset]::UtcNow -lt $deadline)

    if ($null -ne $lastFailure) {
        throw "DESKTOP_CANDIDATE_TEMPORARY_ROOT_CLEANUP_FAILED:$($lastFailure.Exception.Message)"
    }
    throw 'DESKTOP_CANDIDATE_TEMPORARY_ROOT_CLEANUP_FAILED:TIMEOUT'
}

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$headSha = ([string](& git -C $repositoryRoot rev-parse HEAD)).Trim()
if ($LASTEXITCODE -ne 0 -or $headSha -notmatch '^[0-9a-f]{40}$') {
    throw 'DESKTOP_CANDIDATE_TEST_GIT_HEAD_UNAVAILABLE'
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
    throw 'DESKTOP_CANDIDATE_TEST_ARCHIVE_MISSING'
}
$candidateArchiveSha256 = Get-Sha256Hex -LiteralPath $candidateArchiveFull

$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) (
    'lattice-desktop-candidate-' + [Guid]::NewGuid().ToString('N'))
$candidateDirectoryFull = Join-Path $temporaryRoot 'candidate'
$desktopUserDataFolder = Join-Path $temporaryRoot 'desktop-webview2'
$controlLocalApplicationData = Join-Path $temporaryRoot 'control-localappdata'
$isolatedDatabasePath = Join-Path $controlLocalApplicationData 'LATTICE\control\lattice-control.db'
$gatewayReadyPath = Join-Path $temporaryRoot 'gateway-port.txt'
$gatewayModePath = Join-Path $temporaryRoot 'gateway-mode.txt'
$externalNavigationMarker = Join-Path $temporaryRoot 'external-navigation-request.txt'
$captureReadyPath = Join-Path $temporaryRoot 'capture-port.txt'
$captureMarkerPath = Join-Path $temporaryRoot 'external-navigation-capture.txt'
$controlReadyPath = Join-Path $temporaryRoot 'control-port.txt'

$desktopProcess = $null
$gatewayProcess = $null
$controlProcess = $null
$acceptanceResult = $null
$primaryFailure = $null
$cleanupFailure = $null

try {
    [IO.Directory]::CreateDirectory($temporaryRoot) | Out-Null
    [IO.File]::WriteAllText($gatewayModePath, "offline`n", [Text.UTF8Encoding]::new($false))
    Expand-Archive -LiteralPath $candidateArchiveFull -DestinationPath $candidateDirectoryFull

    $candidateExecutable = Join-Path $candidateDirectoryFull 'LATTICE.exe'
    $candidateAssembly = Join-Path $candidateDirectoryFull 'LATTICE.dll'
    $candidateNotice = Join-Path $candidateDirectoryFull 'PORTABLE_RELEASE_CANDIDATE.txt'
    $candidateManifestPath = Join-Path $candidateDirectoryFull 'candidate-manifest.json'
    foreach ($requiredFile in @($candidateExecutable, $candidateAssembly, $candidateNotice, $candidateManifestPath)) {
        if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
            throw 'DESKTOP_CANDIDATE_TEST_REQUIRED_FILE_MISSING'
        }
    }

    try {
        $candidateManifest = Get-Content -LiteralPath $candidateManifestPath -Raw | ConvertFrom-Json
    }
    catch {
        throw 'DESKTOP_CANDIDATE_MANIFEST_INVALID_JSON'
    }
    $schemaVersion = [string](Get-RequiredPropertyValue $candidateManifest 'schema_version')
    $artifactType = [string](Get-RequiredPropertyValue $candidateManifest 'artifact_type')
    $manifestSourceCommit = [string](Get-RequiredPropertyValue $candidateManifest 'source_commit')
    $runtimeIdentifier = [string](Get-RequiredPropertyValue $candidateManifest 'runtime_identifier')
    $selfContained = Get-RequiredPropertyValue $candidateManifest 'self_contained'
    $launchFile = [string](Get-RequiredPropertyValue $candidateManifest 'launch')
    $controlOrigin = [string](Get-RequiredPropertyValue $candidateManifest 'control_origin')
    $manifestExecutableSha256 = [string](Get-RequiredPropertyValue $candidateManifest 'executable_sha256')
    $controlRuntime = Get-RequiredPropertyValue $candidateManifest 'control_runtime'
    $controlRuntimeIdentitySchema = [string](Get-RequiredPropertyValue $controlRuntime 'identity_schema')
    $controlRuntimeProduct = [string](Get-RequiredPropertyValue $controlRuntime 'product')
    $controlRuntimeVersion = [string](Get-RequiredPropertyValue $controlRuntime 'version')
    $controlRuntimeNodeVersion = [string](Get-RequiredPropertyValue $controlRuntime 'node_version')
    $controlRuntimeNodeSha256 = [string](Get-RequiredPropertyValue $controlRuntime 'node_sha256')
    $controlRuntimeExecutable = [string](Get-RequiredPropertyValue $controlRuntime 'executable')
    $controlRuntimeServer = [string](Get-RequiredPropertyValue $controlRuntime 'server')
    $controlRuntimeDatabase = [string](Get-RequiredPropertyValue $controlRuntime 'database')
    if ($schemaVersion -cne 'lattice.control.desktop-portable-candidate.v2' -or
        $artifactType -cne 'PORTABLE_RELEASE_CANDIDATE' -or
        $manifestSourceCommit -cne $headSha -or
        $runtimeIdentifier -cne 'win-x64' -or
        $selfContained -isnot [bool] -or -not $selfContained -or
        $launchFile -cne 'LATTICE.exe' -or
        $controlOrigin -cne 'http://127.0.0.1:4317/' -or
        $manifestExecutableSha256 -cnotmatch '^[0-9a-f]{64}$' -or
        $controlRuntimeIdentitySchema -cne 'lattice.control.runtime-identity.v1' -or
        $controlRuntimeProduct -cne 'LATTICE_CONTROL' -or
        [string]::IsNullOrWhiteSpace($controlRuntimeVersion) -or
        $controlRuntimeNodeVersion -cnotmatch '^v[0-9]+\.[0-9]+\.[0-9]+$' -or
        $controlRuntimeNodeSha256 -cnotmatch '^[0-9a-f]{64}$' -or
        $controlRuntimeExecutable -cne 'control-runtime/node.exe' -or
        $controlRuntimeServer -cne 'control-runtime/apps/lattice-control/src/server.mjs' -or
        $controlRuntimeDatabase -cne '%LOCALAPPDATA%\LATTICE\control\lattice-control.db') {
        throw 'DESKTOP_CANDIDATE_MANIFEST_CONTRACT_MISMATCH'
    }

    $manifestFileEntries = @(Get-RequiredPropertyValue $candidateManifest 'files')
    if ($manifestFileEntries.Count -eq 0 -or $manifestFileEntries.Count -gt 4096) {
        throw 'DESKTOP_CANDIDATE_MANIFEST_FILE_SET_INVALID'
    }
    $expectedFiles = @{}
    foreach ($entry in $manifestFileEntries) {
        $relativePath = [string](Get-RequiredPropertyValue $entry 'path')
        $lengthText = [string](Get-RequiredPropertyValue $entry 'length')
        $sha256 = [string](Get-RequiredPropertyValue $entry 'sha256')
        if ([string]::IsNullOrWhiteSpace($relativePath) -or
            [IO.Path]::IsPathRooted($relativePath) -or
            $relativePath.Contains('\') -or
            $relativePath -match '(^|/)\.\.?(/|$)' -or
            $relativePath -eq 'candidate-manifest.json' -or
            $lengthText -notmatch '^(0|[1-9][0-9]*)$' -or
            $sha256 -cnotmatch '^[0-9a-f]{64}$' -or
            $expectedFiles.ContainsKey($relativePath)) {
            throw 'DESKTOP_CANDIDATE_MANIFEST_FILE_ENTRY_INVALID'
        }
        $expectedFiles[$relativePath] = [PSCustomObject]@{
            Length = [long]::Parse($lengthText, [Globalization.CultureInfo]::InvariantCulture)
            Sha256 = $sha256
        }
    }
    foreach ($requiredRelativePath in @(
        'LATTICE.exe',
        'LATTICE.dll',
        'PORTABLE_RELEASE_CANDIDATE.txt',
        $controlRuntimeExecutable,
        $controlRuntimeServer,
        'control-runtime/apps/lattice-control/runtime-identity.json')) {
        if (-not $expectedFiles.ContainsKey($requiredRelativePath)) {
            throw 'DESKTOP_CANDIDATE_MANIFEST_CORE_FILE_MISSING'
        }
    }

    $actualPackageFiles = @(Get-ChildItem -LiteralPath $candidateDirectoryFull -File -Recurse |
        Where-Object { $_.FullName -ne $candidateManifestPath })
    if ($actualPackageFiles.Count -ne $expectedFiles.Count) {
        throw 'DESKTOP_CANDIDATE_PACKAGE_FILE_COUNT_MISMATCH'
    }
    foreach ($actualFile in $actualPackageFiles) {
        $relativePath = $actualFile.FullName.Substring($candidateDirectoryFull.Length + 1).Replace('\', '/')
        if (-not $expectedFiles.ContainsKey($relativePath)) {
            throw "DESKTOP_CANDIDATE_PACKAGE_UNDECLARED_FILE:$relativePath"
        }
        $expectedFile = $expectedFiles[$relativePath]
        $actualSha256 = Get-Sha256Hex -LiteralPath $actualFile.FullName
        if ($actualFile.Length -ne $expectedFile.Length -or $actualSha256 -cne $expectedFile.Sha256) {
            throw "DESKTOP_CANDIDATE_PACKAGE_FILE_MISMATCH:$relativePath"
        }
    }
    if ($manifestExecutableSha256 -cne $expectedFiles['LATTICE.exe'].Sha256) {
        throw 'DESKTOP_CANDIDATE_EXECUTABLE_HASH_MISMATCH'
    }
    if ($controlRuntimeNodeSha256 -cne $expectedFiles[$controlRuntimeExecutable].Sha256) {
        throw 'DESKTOP_CANDIDATE_CONTROL_RUNTIME_HASH_MISMATCH'
    }

    $packageLeaks = @(Get-ChildItem -LiteralPath $candidateDirectoryFull -Directory -Recurse -Force |
        Where-Object { $_.Name -eq 'EBWebView' -or $_.Name -like '*.WebView2' })
    if ($packageLeaks.Count -ne 0) {
        throw 'DESKTOP_CANDIDATE_WEBVIEW_USER_DATA_LEAKED_INTO_PACKAGE'
    }
    if (Test-Path -LiteralPath $desktopUserDataFolder) {
        throw 'DESKTOP_CANDIDATE_TEST_WEBVIEW_DATA_NOT_FRESH'
    }

    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    $nodePath = (Get-Command node -CommandType Application -ErrorAction Stop).Source

    $gatewayStart = [Diagnostics.ProcessStartInfo]::new()
    $gatewayStart.FileName = $nodePath
    $gatewayStart.WorkingDirectory = $repositoryRoot
    $gatewayStart.UseShellExecute = $false
    $gatewayStart.CreateNoWindow = $true
    $gatewayStart.Arguments = 'apps/lattice-control/test/fixtures/desktop-external-redirect.mjs'
    $gatewayStart.EnvironmentVariables['LATTICE_DESKTOP_GATEWAY_READY'] = $gatewayReadyPath
    $gatewayStart.EnvironmentVariables['LATTICE_DESKTOP_GATEWAY_MODE'] = $gatewayModePath
    $gatewayStart.EnvironmentVariables['LATTICE_DESKTOP_REDIRECT_MARKER'] = $externalNavigationMarker
    $gatewayStart.EnvironmentVariables['LATTICE_DESKTOP_CAPTURE_READY'] = $captureReadyPath
    $gatewayStart.EnvironmentVariables['LATTICE_DESKTOP_CAPTURE_MARKER'] = $captureMarkerPath
    $gatewayStart.EnvironmentVariables['LATTICE_DESKTOP_BACKEND_PORT'] = $controlReadyPath
    $gatewayProcess = [Diagnostics.Process]::Start($gatewayStart)
    if ($null -eq $gatewayProcess) {
        throw 'DESKTOP_CANDIDATE_GATEWAY_START_FAILED'
    }
    $gatewayPort = ConvertTo-LoopbackPort (
        Wait-ProcessTextFile $gatewayReadyPath $gatewayProcess $ReconnectTimeoutSeconds 'DESKTOP_CANDIDATE_GATEWAY_READY_FAILED'
    ) 'DESKTOP_CANDIDATE_GATEWAY_PORT_INVALID'
    $capturePort = ConvertTo-LoopbackPort (
        Wait-ProcessTextFile $captureReadyPath $gatewayProcess $ReconnectTimeoutSeconds 'DESKTOP_CANDIDATE_CAPTURE_READY_FAILED'
    ) 'DESKTOP_CANDIDATE_CAPTURE_PORT_INVALID'
    if ($gatewayPort -eq $capturePort) {
        throw 'DESKTOP_CANDIDATE_TEST_PORT_COLLISION'
    }

    $desktopStart = [Diagnostics.ProcessStartInfo]::new()
    $desktopStart.FileName = $candidateExecutable
    $desktopStart.WorkingDirectory = $candidateDirectoryFull
    $desktopStart.UseShellExecute = $false
    $desktopStart.Arguments = "--url http://127.0.0.1:$gatewayPort/"
    $desktopStart.EnvironmentVariables['WEBVIEW2_USER_DATA_FOLDER'] = $desktopUserDataFolder
    $desktopProcess = [Diagnostics.Process]::Start($desktopStart)
    if ($null -eq $desktopProcess) {
        throw 'DESKTOP_CANDIDATE_PROCESS_START_FAILED'
    }
    $startedAt = [DateTimeOffset]::UtcNow

    $offlineStatus = Wait-ConnectionStatus $desktopProcess $offlineConnectionStatus $ReconnectTimeoutSeconds
    Wait-ConnectionItemStatus $desktopProcess 'offline' $ReconnectTimeoutSeconds | Out-Null
    $userDataDeadline = [DateTimeOffset]::UtcNow.AddSeconds(15)
    while (-not (Test-Path -LiteralPath $desktopUserDataFolder -PathType Container)) {
        if ([DateTimeOffset]::UtcNow -ge $userDataDeadline) {
            throw 'DESKTOP_CANDIDATE_WEBVIEW_USER_DATA_MISSING'
        }
        Start-Sleep -Milliseconds 250
    }
    $candidatePrefix = $candidateDirectoryFull.TrimEnd([IO.Path]::DirectorySeparatorChar) +
        [IO.Path]::DirectorySeparatorChar
    $desktopUserDataFull = [IO.Path]::GetFullPath($desktopUserDataFolder)
    if ($desktopUserDataFull.StartsWith($candidatePrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'DESKTOP_CANDIDATE_WEBVIEW_USER_DATA_INSIDE_PACKAGE'
    }

    [IO.File]::WriteAllText($gatewayModePath, "redirect`n", [Text.UTF8Encoding]::new($false))
    $externalRequest = Wait-ProcessTextFile `
        $externalNavigationMarker `
        $gatewayProcess `
        $ReconnectTimeoutSeconds `
        'DESKTOP_CANDIDATE_EXTERNAL_REDIRECT_NOT_OBSERVED'
    $externalItemStatus = Wait-ConnectionItemStatus `
        $desktopProcess `
        'external_navigation_blocked' `
        $ReconnectTimeoutSeconds
    Start-Sleep -Milliseconds 500
    $externalNavigationBlocked = $externalItemStatus -eq 'external_navigation_blocked' -and
        -not (Test-Path -LiteralPath $captureMarkerPath)
    if (-not $externalNavigationBlocked -or $externalRequest -notmatch '^GET\s+/') {
        throw 'DESKTOP_CANDIDATE_EXTERNAL_NAVIGATION_NOT_BLOCKED'
    }
    [IO.File]::WriteAllText($gatewayModePath, "offline`n", [Text.UTF8Encoding]::new($false))
    Wait-ConnectionItemStatus $desktopProcess 'offline' $ReconnectTimeoutSeconds | Out-Null

    $controlStart = [Diagnostics.ProcessStartInfo]::new()
    $controlStart.FileName = $nodePath
    $controlStart.WorkingDirectory = $repositoryRoot
    $controlStart.UseShellExecute = $false
    $controlStart.CreateNoWindow = $true
    $controlStart.Arguments = 'apps/lattice-control/test/fixtures/desktop-isolated-control.mjs'
    $controlStart.EnvironmentVariables['LOCALAPPDATA'] = $controlLocalApplicationData
    $controlStart.EnvironmentVariables['LATTICE_DESKTOP_CONTROL_READY'] = $controlReadyPath
    $controlProcess = [Diagnostics.Process]::Start($controlStart)
    if ($null -eq $controlProcess) {
        throw 'DESKTOP_CANDIDATE_CONTROL_START_FAILED'
    }
    $controlPort = ConvertTo-LoopbackPort (
        Wait-ProcessTextFile $controlReadyPath $controlProcess $ReconnectTimeoutSeconds 'DESKTOP_CANDIDATE_CONTROL_READY_FAILED'
    ) 'DESKTOP_CANDIDATE_CONTROL_PORT_INVALID'
    if ($controlPort -eq $gatewayPort -or $controlPort -eq $capturePort) {
        throw 'DESKTOP_CANDIDATE_TEST_PORT_COLLISION'
    }
    if (-not (Test-Path -LiteralPath $isolatedDatabasePath -PathType Leaf)) {
        throw 'DESKTOP_CANDIDATE_ISOLATED_DATABASE_MISSING'
    }
    $response = Invoke-WebRequest `
        -Uri "http://127.0.0.1:$controlPort/api/four-core" `
        -TimeoutSec 5 `
        -UseBasicParsing
    if ($response.StatusCode -ne 200) {
        throw 'DESKTOP_CANDIDATE_CONTROL_API_FAILED'
    }

    [IO.File]::WriteAllText($gatewayModePath, "proxy`n", [Text.UTF8Encoding]::new($false))
    $connectedStatus = Wait-ConnectionStatus $desktopProcess $connectedConnectionStatus $ReconnectTimeoutSeconds
    Wait-ConnectionItemStatus $desktopProcess 'connected' $ReconnectTimeoutSeconds | Out-Null

    $monitorStartedAt = [DateTimeOffset]::UtcNow
    $lifetimeDeadline = $monitorStartedAt.AddSeconds($MinimumLifetimeSeconds)
    while ([DateTimeOffset]::UtcNow -lt $lifetimeDeadline) {
        if ($desktopProcess.HasExited) {
            throw 'DESKTOP_CANDIDATE_EXITED_BEFORE_MINIMUM_LIFETIME'
        }
        if ($gatewayProcess.HasExited -or $controlProcess.HasExited) {
            throw 'DESKTOP_CANDIDATE_OWNED_ENDPOINT_EXITED_BEFORE_MINIMUM_LIFETIME'
        }
        if (Test-Path -LiteralPath $captureMarkerPath) {
            throw 'DESKTOP_CANDIDATE_EXTERNAL_CAPTURE_OBSERVED'
        }
        Start-Sleep -Milliseconds 500
    }
    $captureReceived = Test-Path -LiteralPath $captureMarkerPath
    if ($captureReceived) {
        throw 'DESKTOP_CANDIDATE_EXTERNAL_CAPTURE_OBSERVED'
    }
    if ((Get-ConnectionItemStatus $desktopProcess) -ne 'connected' -or
        -not (Test-Path -LiteralPath $isolatedDatabasePath -PathType Leaf)) {
        throw 'DESKTOP_CANDIDATE_FINAL_ENDPOINT_IDENTITY_FAILED'
    }
    $desktopTotalLifetimeSeconds = [Math]::Floor(
        ([DateTimeOffset]::UtcNow - $startedAt).TotalSeconds)
    if ($desktopTotalLifetimeSeconds -lt $MinimumLifetimeSeconds) {
        throw 'DESKTOP_CANDIDATE_TOTAL_LIFETIME_TOO_SHORT'
    }

    $acceptanceResult = [ordered]@{
        result = 'PASS'
        source_commit = $manifestSourceCommit
        artifact_type = $artifactType
        candidate_archive = $candidateArchiveFull
        candidate_archive_sha256 = $candidateArchiveSha256
        tested_from_extracted_archive = $true
        manifest_file_count = $expectedFiles.Count
        minimum_lifetime_seconds = $MinimumLifetimeSeconds
        desktop_total_lifetime_seconds = $desktopTotalLifetimeSeconds
        owned_process_monitor_seconds = $MinimumLifetimeSeconds
        offline_status = $offlineStatus
        connected_status = $connectedStatus
        external_navigation_request = $externalRequest
        external_navigation_blocked = $externalNavigationBlocked
        external_navigation_capture_received = $captureReceived
        control_database_scope = 'DISPOSABLE_TEST_ONLY'
        control_database_path = $isolatedDatabasePath
        control_test_port = $controlPort
        desktop_gateway_test_port = $gatewayPort
        production_control_port_used = $false
        webview_user_data = $desktopUserDataFull
        webview_user_data_override = 'WEBVIEW2_USER_DATA_FOLDER'
        webview_data_outside_candidate = $true
        desktop_process_alive = -not $desktopProcess.HasExited
        owned_endpoint_processes_alive = -not $gatewayProcess.HasExited -and -not $controlProcess.HasExited
    }
}
catch {
    $primaryFailure = $_
}
finally {
    try {
        if ($null -ne $desktopProcess -and -not $desktopProcess.HasExited) {
            try {
                $desktopProcess.CloseMainWindow() | Out-Null
                $desktopProcess.WaitForExit(10000) | Out-Null
            }
            catch {
                # A validated kill fallback below still proves cleanup.
            }
            if (-not $desktopProcess.HasExited) {
                Stop-OwnedProcess $desktopProcess
            }
        }
    }
    catch {
        $cleanupFailure = $_
    }
    try {
        Stop-OwnedProcess $controlProcess
    }
    catch {
        if ($null -eq $cleanupFailure) {
            $cleanupFailure = $_
        }
    }
    try {
        Stop-OwnedProcess $gatewayProcess
    }
    catch {
        if ($null -eq $cleanupFailure) {
            $cleanupFailure = $_
        }
    }

    try {
        $temporaryRootFull = [IO.Path]::GetFullPath($temporaryRoot)
        $temporaryPrefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
            [IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
        if (-not $temporaryRootFull.StartsWith($temporaryPrefix, [StringComparison]::OrdinalIgnoreCase) -or
            -not [IO.Path]::GetFileName($temporaryRootFull).StartsWith(
                'lattice-desktop-candidate-',
                [StringComparison]::Ordinal)) {
            throw 'DESKTOP_CANDIDATE_TEMPORARY_ROOT_INVALID'
        }
        Remove-TemporaryRootWithRetry -LiteralPath $temporaryRootFull
    }
    catch {
        if ($null -eq $cleanupFailure) {
            $cleanupFailure = $_
        }
    }
}

if ($null -ne $primaryFailure) {
    if ($null -ne $cleanupFailure) {
        Write-Warning `
            "DESKTOP_CANDIDATE_CLEANUP_FAILED_AFTER_PRIMARY:$($cleanupFailure.Exception.Message)" `
            -WarningAction Continue
    }
    throw $primaryFailure
}
if ($null -ne $cleanupFailure) {
    throw $cleanupFailure
}
if ($null -eq $acceptanceResult) {
    throw 'DESKTOP_CANDIDATE_ACCEPTANCE_RESULT_MISSING'
}
$acceptanceResult | ConvertTo-Json -Depth 4
