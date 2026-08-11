[CmdletBinding()]
param(
    [switch]$RunLatticeDeliveryHook,
    [switch]$RunFullChainAcceptanceHook,
    [switch]$RunTask038AcceptanceHook,
    [switch]$RunTask038TunnelHook,
    [string]$Task038OfficialCodexExecutable,
    [string]$Task038CodexAuthHome,
    [string]$Task038TunnelClientExecutable,
    [string]$Task038TunnelProfileDirectory,
    [ValidatePattern('^[a-z0-9][a-z0-9-]{0,63}$')]
    [string]$Task038TunnelProfileName = 'lattice-local',
    [string]$Task038EvidenceDirectory,
    [string]$Task038CanonicalAcceptanceReceipt,
    [ValidatePattern('^[0-9a-fA-F]{64}$')]
    [string]$Task038CanonicalAcceptanceReceiptSha256,
    [switch]$MemoryOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$nativeIdentityHelperPath = Join-Path $PSScriptRoot 'windows-native-path-identity.ps1'
$nativeIdentityHelperItem = Get-Item -LiteralPath $nativeIdentityHelperPath -Force -ErrorAction SilentlyContinue
if (
    $null -eq $nativeIdentityHelperItem -or
    $nativeIdentityHelperItem.PSIsContainer -or
    -not ($nativeIdentityHelperItem -is [IO.FileInfo]) -or
    ($nativeIdentityHelperItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
) {
    throw 'TASK019_WINDOWS_NATIVE_IDENTITY_HELPER_REJECTED'
}
try {
    $nativeIdentityHelperSource = [Text.UTF8Encoding]::new($false, $true).GetString(
        [IO.File]::ReadAllBytes($nativeIdentityHelperItem.FullName)
    )
    . ([scriptblock]::Create($nativeIdentityHelperSource))
    Initialize-LatticeWindowsNativePathIdentity
}
catch {
    throw 'TASK019_WINDOWS_NATIVE_IDENTITY_HELPER_REJECTED'
}

$postgresBin = 'C:\Program Files\PostgreSQL\17\bin'
$requiredExecutables = @(
    'initdb.exe',
    'pg_ctl.exe',
    'pg_isready.exe',
    'postgres.exe'
)
$serviceName = 'postgresql-x64-17'
$markerName = '.lattice-task019-disposable.json'
$expectedPostgresVersion = '17.10'
$harnessUser = 'task019_harness'
$environmentNames = @(
    'LATTICE_TASK019_LIVE',
    'LATTICE_TASK019_PHASE',
    'LATTICE_TASK019_HOST',
    'LATTICE_TASK019_PORT',
    'LATTICE_TASK019_PASSWORD',
    'LATTICE_TASK019_RUN_ID',
    'LATTICE_TASK019_EXPECTED_UUID',
    'LATTICE_TASK019_EXPECTED_MANIFEST',
    'LATTICE_TASK038_POSTGRES_PASSWORD',
    'LATTICE_WRITER_LEASE_MIGRATOR_URL',
    'LATTICE_WRITER_LEASE_RUNTIME_URL',
    'LATTICE_WRITER_LEASE_ADMIN_URL',
    'LATTICE_STORE_PROFILE_LIVE',
    'LATTICE_STORE_PROFILE_EXPECTED',
    'LATTICE_STORE_PROFILE_RUNTIME_URL',
    'LATTICE_STORE_PROFILE_MIGRATOR_URL'
)

function Get-CanonicalPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $trimCharacters = [char[]]@(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
    return [System.IO.Path]::GetFullPath($Path).TrimEnd($trimCharacters)
}

function Test-ExactPath {
    param(
        [Parameter(Mandatory = $true)][string]$Actual,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    return [string]::Equals(
        (Get-CanonicalPath -Path $Actual),
        (Get-CanonicalPath -Path $Expected),
        [System.StringComparison]::OrdinalIgnoreCase
    )
}

function Assert-NoReparseAncestor {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Boundary
    )

    $canonicalPath = Get-CanonicalPath -Path $Path
    $canonicalBoundary = Get-CanonicalPath -Path $Boundary
    $boundaryPrefix = $canonicalBoundary + [System.IO.Path]::DirectorySeparatorChar
    if (-not (Test-ExactPath -Actual $canonicalPath -Expected $canonicalBoundary) -and
        -not $canonicalPath.StartsWith($boundaryPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'TASK-019 path is outside the repository boundary.'
    }

    $current = $canonicalPath
    while ($true) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                throw 'TASK-019 path has an existing reparse-point ancestor.'
            }
        }
        if (Test-ExactPath -Actual $current -Expected $canonicalBoundary) {
            break
        }
        $parent = Split-Path -Parent $current
        if ([string]::IsNullOrWhiteSpace($parent) -or (Test-ExactPath -Actual $parent -Expected $current)) {
            throw 'TASK-019 path ancestry could not be proved.'
        }
        $current = $parent
    }
}

function Get-LatticeDeliveryHookPath {
    param(
        [Parameter(Mandatory = $true)][string]$ScriptDirectory,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $canonicalScriptDirectory = Get-CanonicalPath -Path $ScriptDirectory
    $expectedPath = Get-CanonicalPath -Path (Join-Path $canonicalScriptDirectory 'run-lattice-delivery.ps1')
    if (-not (Test-ExactPath -Actual (Split-Path -Parent $expectedPath) -Expected $canonicalScriptDirectory)) {
        throw 'TASK019_DELIVERY_HOOK_NOT_EXACT_SIBLING'
    }

    Assert-NoReparseAncestor -Path $expectedPath -Boundary $RepositoryRoot
    $item = Get-Item -LiteralPath $expectedPath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [System.IO.FileInfo]) -or
        ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
        -not (Test-Path -LiteralPath $expectedPath -PathType Leaf) -or
        -not (Test-ExactPath -Actual $item.FullName -Expected $expectedPath)
    ) {
        throw 'TASK019_DELIVERY_HOOK_NOT_REGULAR_LEAF'
    }

    return $expectedPath
}

function Get-LatticeFullChainAcceptanceHookPath {
    param(
        [Parameter(Mandatory = $true)][string]$ScriptDirectory,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $canonicalScriptDirectory = Get-CanonicalPath -Path $ScriptDirectory
    $expectedPath = Get-CanonicalPath -Path (Join-Path $canonicalScriptDirectory 'run-task037-full-chain-verification.ps1')
    if (-not (Test-ExactPath -Actual (Split-Path -Parent $expectedPath) -Expected $canonicalScriptDirectory)) {
        throw 'TASK037_FULL_CHAIN_HOOK_NOT_EXACT_SIBLING'
    }

    Assert-NoReparseAncestor -Path $expectedPath -Boundary $RepositoryRoot
    $item = Get-Item -LiteralPath $expectedPath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [System.IO.FileInfo]) -or
        ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
        -not (Test-Path -LiteralPath $expectedPath -PathType Leaf) -or
        -not (Test-ExactPath -Actual $item.FullName -Expected $expectedPath)
    ) {
        throw 'TASK037_FULL_CHAIN_HOOK_NOT_REGULAR_LEAF'
    }

    return $expectedPath
}

function Test-NoReparseTree {
    param([Parameter(Mandatory = $true)][string]$Root)

    $rootItem = Get-Item -LiteralPath $Root -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $rootItem -or
        -not $rootItem.PSIsContainer -or
        -not ($rootItem -is [System.IO.DirectoryInfo]) -or
        ($rootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)
    ) {
        return $false
    }
    $pending = [Collections.Generic.Queue[System.IO.DirectoryInfo]]::new()
    $pending.Enqueue($rootItem)
    try {
        while ($pending.Count -gt 0) {
            $directory = $pending.Dequeue()
            foreach ($item in $directory.EnumerateFileSystemInfos()) {
                if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                    return $false
                }
                if ($item -is [System.IO.DirectoryInfo]) {
                    $pending.Enqueue($item)
                }
            }
        }
    }
    catch {
        return $false
    }
    return $true
}

function Remove-Task038DisposableCodexAuth {
    param(
        [Parameter(Mandatory = $true)][string]$ClusterRoot,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $canonicalCluster = Get-CanonicalPath -Path $ClusterRoot
    $codexHomePath = Get-CanonicalPath -Path (Join-Path $canonicalCluster 'task038-tunnel-codex-home')
    Assert-NoReparseAncestor -Path $codexHomePath -Boundary $RepositoryRoot
    $homeItem = Get-Item -LiteralPath $codexHomePath -Force -ErrorAction SilentlyContinue
    if ($null -eq $homeItem) { return }
    if (
        -not $homeItem.PSIsContainer -or
        -not ($homeItem -is [System.IO.DirectoryInfo]) -or
        ($homeItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
        -not (Test-ExactPath -Actual (Split-Path -Parent $codexHomePath) -Expected $canonicalCluster)
    ) {
        throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    }
    $homeMarker = Join-Path $codexHomePath '.lattice-codex-home-v1'
    $ownerMarker = Join-Path $codexHomePath '.lattice-task038-tunnel-owner-v1'
    foreach ($marker in @($homeMarker, $ownerMarker)) {
        $markerItem = Get-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue
        if (
            $null -eq $markerItem -or
            $markerItem.PSIsContainer -or
            -not ($markerItem -is [System.IO.FileInfo]) -or
            ($markerItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)
        ) {
            throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
        }
    }
    if (
        [IO.File]::ReadAllText($homeMarker, [Text.Encoding]::UTF8) -cne "lattice.codex-home.v1`n" -or
        [IO.File]::ReadAllText($ownerMarker, [Text.Encoding]::UTF8) -cnotmatch
            '^lattice\.task038-tunnel-home\.v1:[0-9a-f]{32}\r?\n$'
    ) {
        throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    }
    $authPath = Join-Path $codexHomePath 'auth.json'
    if (-not (Test-Path -LiteralPath $authPath)) { return }
    Assert-NoReparseAncestor -Path $authPath -Boundary $RepositoryRoot
    $authItem = Get-Item -LiteralPath $authPath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $authItem -or
        $authItem.PSIsContainer -or
        -not ($authItem -is [System.IO.FileInfo]) -or
        ($authItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    }
    [IO.File]::Delete($authPath)
    if (Test-Path -LiteralPath $authPath) {
        throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    }
}

function Get-LatticeTask038AcceptanceHookPath {
    param(
        [Parameter(Mandatory = $true)][string]$ScriptDirectory,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $canonicalScriptDirectory = Get-CanonicalPath -Path $ScriptDirectory
    $expectedPath = Get-CanonicalPath -Path (Join-Path $canonicalScriptDirectory 'run-task038-task-submit.ps1')
    if (-not (Test-ExactPath -Actual (Split-Path -Parent $expectedPath) -Expected $canonicalScriptDirectory)) {
        throw 'TASK038_ACCEPTANCE_HOOK_NOT_EXACT_SIBLING'
    }
    Assert-NoReparseAncestor -Path $expectedPath -Boundary $RepositoryRoot
    $item = Get-Item -LiteralPath $expectedPath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [System.IO.FileInfo]) -or
        ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
        -not (Test-Path -LiteralPath $expectedPath -PathType Leaf) -or
        -not (Test-ExactPath -Actual $item.FullName -Expected $expectedPath)
    ) {
        throw 'TASK038_ACCEPTANCE_HOOK_NOT_REGULAR_LEAF'
    }
    return $expectedPath
}

function Get-LatticeTask038TunnelHookPath {
    param(
        [Parameter(Mandatory = $true)][string]$ScriptDirectory,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $canonicalScriptDirectory = Get-CanonicalPath -Path $ScriptDirectory
    $expectedPath = Get-CanonicalPath -Path (Join-Path $canonicalScriptDirectory 'start-chatgpt-mcp-tunnel.ps1')
    Assert-NoReparseAncestor -Path $expectedPath -Boundary $RepositoryRoot
    $item = Get-Item -LiteralPath $expectedPath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or $item.PSIsContainer -or
        -not ($item -is [System.IO.FileInfo]) -or
        ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
        -not (Test-Path -LiteralPath $expectedPath -PathType Leaf) -or
        -not (Test-ExactPath -Actual $item.FullName -Expected $expectedPath)
    ) {
        throw 'TASK038_TUNNEL_HOOK_NOT_REGULAR_LEAF'
    }
    return $expectedPath
}

function Test-PgCtlStatusCodeIsStopped {
    param([Parameter(Mandatory = $true)][int]$StatusCode)

    return ($StatusCode -eq 3)
}

function Test-StoreProfileLiveGateOutput {
    param(
        [Parameter(Mandatory = $true)][int]$ExitCode,
        [Parameter(Mandatory = $true)][object[]]$Output,
        [Parameter(Mandatory = $true)][string]$ExpectedProfile
    )

    $allowedProfiles = @('V3', 'V3_MEMORY_V2', 'V3_MEMORY_V2_WRITER_LEASE_V1')
    if ($ExpectedProfile -notin $allowedProfiles) {
        return $false
    }
    $text = @($Output | ForEach-Object { [string]$_ }) -join "`n"
    $escapedProfile = [regex]::Escape($ExpectedProfile)
    $passPattern = "(?m)(?:^|[^\S\r\n])PASS: Store live profile $escapedProfile accepted with exact fail-closed matrix[ `t]*$"
    $skipPattern = '(?m)(?:^|[^\S\r\n])SKIP:'
    return (
        $ExitCode -eq 0 -and
        $text -match $passPattern -and
        $text -notmatch $skipPattern
    )
}

function Get-StoreProfileForLiveSuitePhase {
    param(
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)][string]$SuiteName
    )

    if ($Phase -eq 'initial' -and $SuiteName -eq 'store') {
        return 'V3'
    }
    if ($Phase -eq 'initial' -and $SuiteName -eq 'memory') {
        return 'V3_MEMORY_V2'
    }
    return $null
}

function Invoke-HarnessSelfTest {
    if (-not (Test-PgCtlStatusCodeIsStopped -StatusCode 3)) {
        throw 'TASK-019 stopped-state contract rejected exit 3.'
    }
    foreach ($statusCode in @(0, 1, 2, 4, 5, 127)) {
        if (Test-PgCtlStatusCodeIsStopped -StatusCode $statusCode) {
            throw 'TASK-019 stopped-state contract accepted an unknown status.'
        }
    }
    $profilePass = @(
        'test postgres_setup::tests::live_store_profile ... PASS: Store live profile V3 accepted with exact fail-closed matrix'
    )
    if (-not (Test-StoreProfileLiveGateOutput -ExitCode 0 -Output $profilePass -ExpectedProfile 'V3')) {
        throw 'TASK019_STORE_PROFILE_OUTPUT_SELF_TEST_REJECTED_PASS'
    }
    foreach ($rejected in @(
        [pscustomobject]@{ ExitCode = 1; Output = $profilePass; Profile = 'V3' },
        [pscustomobject]@{ ExitCode = 0; Output = @('SKIP: LATTICE_STORE_PROFILE_LIVE is not enabled'); Profile = 'V3' },
        [pscustomobject]@{ ExitCode = 0; Output = $profilePass; Profile = 'V3_MEMORY_V2' },
        [pscustomobject]@{ ExitCode = 0; Output = $profilePass; Profile = 'UNKNOWN' }
    )) {
        if (Test-StoreProfileLiveGateOutput `
                -ExitCode $rejected.ExitCode `
                -Output $rejected.Output `
                -ExpectedProfile $rejected.Profile) {
            throw 'TASK019_STORE_PROFILE_OUTPUT_SELF_TEST_ACCEPTED_REJECTION'
        }
    }
    if (
        (Get-StoreProfileForLiveSuitePhase -Phase 'initial' -SuiteName 'store') -ne 'V3' -or
        (Get-StoreProfileForLiveSuitePhase -Phase 'initial' -SuiteName 'memory') -ne 'V3_MEMORY_V2' -or
        $null -ne (Get-StoreProfileForLiveSuitePhase -Phase 'restart' -SuiteName 'store') -or
        $null -ne (Get-StoreProfileForLiveSuitePhase -Phase 'initial' -SuiteName 'unknown')
    ) {
        throw 'TASK019_STORE_PROFILE_PHASE_MAPPING_SELF_TEST_REJECTED'
    }
    Write-Output 'TASK019_HARNESS_SELF_TEST=PASS'
}

function Invoke-NativeChecked {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Operation
    )

    $stdoutPath = Join-Path $clusterRoot '.native-stdout.log'
    $stderrPath = Join-Path $clusterRoot '.native-stderr.log'
    Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    $process = $null
    $nativeExitCode = $null
    try {
        $startParameters = @{
            FilePath = $Executable
            ArgumentList = $Arguments
            RedirectStandardOutput = $stdoutPath
            RedirectStandardError = $stderrPath
            WindowStyle = 'Hidden'
            PassThru = $true
        }
        $process = Start-Process @startParameters
        $null = $process.Handle
        $process.WaitForExit()
        $nativeExitCode = $process.ExitCode
    }
    finally {
        if ($null -ne $process) {
            $process.Dispose()
        }
        Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    }
    if ($null -eq $nativeExitCode -or $nativeExitCode -ne 0) {
        throw "$Operation failed with exit code $nativeExitCode. Native output was suppressed."
    }
}

function Get-PgIsReadyExitCode {
    param([Parameter(Mandatory = $true)][string]$PgIsReady)

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $null = & $PgIsReady '-h' '127.0.0.1' '-p' '5432' '-t' '2' '-q' 2>&1
        return [int]$LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
}

function Get-InstalledPostgresSnapshot {
    param([Parameter(Mandatory = $true)][string]$PgIsReady)

    $service = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
    return [pscustomobject]@{
        ServicePresent = ($null -ne $service)
        ServiceStatus = if ($null -eq $service) { 'ABSENT' } else { [string]$service.Status }
        PgIsReady5432 = Get-PgIsReadyExitCode -PgIsReady $PgIsReady
    }
}

function Test-SameInstalledPostgresSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Before,
        [Parameter(Mandatory = $true)]$After
    )

    return (
        $Before.ServicePresent -eq $After.ServicePresent -and
        $Before.ServiceStatus -eq $After.ServiceStatus -and
        $Before.PgIsReady5432 -eq $After.PgIsReady5432
    )
}

function New-OneTimePassword {
    $bytes = New-Object byte[] 48
    $generator = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $generator.GetBytes($bytes)
        return [Convert]::ToBase64String($bytes)
    }
    finally {
        $generator.Dispose()
        [Array]::Clear($bytes, 0, $bytes.Length)
    }
}

function Get-UnreservedLoopbackPort {
    do {
        $listener = [System.Net.Sockets.TcpListener]::new(
            [System.Net.IPAddress]::Loopback,
            0
        )
        try {
            $listener.Start()
            $candidate = ([System.Net.IPEndPoint]$listener.LocalEndpoint).Port
        }
        finally {
            $listener.Stop()
        }
    } while ($candidate -eq 5432)

    return [int]$candidate
}

function Set-HarnessEnvironment {
    param(
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)][string]$HostName,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$RunId
    )

    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_LIVE', '1', 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PHASE', $Phase, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_HOST', $HostName, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PORT', [string]$Port, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PASSWORD', $Password, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_RUN_ID', $RunId, 'Process')
}

function Invoke-StoreProfileLiveGate {
    param(
        [Parameter(Mandatory = $true)][string]$Cargo,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$ExpectedProfile,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$RunId
    )

    if ($ExpectedProfile -notin @('V3', 'V3_MEMORY_V2', 'V3_MEMORY_V2_WRITER_LEASE_V1')) {
        throw 'TASK019_STORE_PROFILE_EXPECTATION_REJECTED'
    }
    if ($RunId -notmatch '^[0-9a-f]{32}$') {
        throw 'TASK019_STORE_PROFILE_RUN_ID_REJECTED'
    }

    $databaseName = 'lattice_task019_' + $RunId.Substring(0, 8) + '_base'
    $encodedPassword = [Uri]::EscapeDataString($Password)
    $profileEnvironment = [ordered]@{
        LATTICE_STORE_PROFILE_LIVE = '1'
        LATTICE_STORE_PROFILE_EXPECTED = $ExpectedProfile
        LATTICE_STORE_PROFILE_RUNTIME_URL = ('postgresql://lattice_runtime_login:{0}@127.0.0.1:{1}/{2}' -f $encodedPassword, $Port, $databaseName)
        LATTICE_STORE_PROFILE_MIGRATOR_URL = ('postgresql://lattice_migrator_login:{0}@127.0.0.1:{1}/{2}' -f $encodedPassword, $Port, $databaseName)
    }
    $original = @{}
    $stdoutPath = Join-Path $clusterRoot ".cargo-store-profile-$ExpectedProfile-stdout.log"
    $stderrPath = Join-Path $clusterRoot ".cargo-store-profile-$ExpectedProfile-stderr.log"
    $process = $null
    $testExitCode = $null
    $testOutput = @()
    Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    try {
        foreach ($entry in $profileEnvironment.GetEnumerator()) {
            $original[[string]$entry.Key] = [Environment]::GetEnvironmentVariable([string]$entry.Key, 'Process')
            [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
        }
        $process = Start-Process -FilePath $Cargo -ArgumentList @(
            'test',
            '-p', 'lattice-postgres-store',
            '--lib',
            'live_store_profile_accepts_exact_profiles_and_rejects_writer_lease_drift_when_provisioned',
            '--locked',
            '--',
            '--nocapture',
            '--test-threads=1'
        ) -WorkingDirectory $RepositoryRoot -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath -WindowStyle Hidden -PassThru
        $null = $process.Handle
        $process.WaitForExit()
        $testExitCode = $process.ExitCode
        if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) {
            $testOutput += @(Get-Content -LiteralPath $stdoutPath -Encoding utf8)
        }
        if (Test-Path -LiteralPath $stderrPath -PathType Leaf) {
            $testOutput += @(Get-Content -LiteralPath $stderrPath -Encoding utf8)
        }
    }
    finally {
        if ($null -ne $process) {
            $process.Dispose()
        }
        foreach ($entry in $original.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, $entry.Value, 'Process')
        }
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $path) {
                throw 'TASK019_STORE_PROFILE_OUTPUT_DELETE_FAILED'
            }
        }
    }

    if (-not (Test-StoreProfileLiveGateOutput `
            -ExitCode $testExitCode `
            -Output $testOutput `
            -ExpectedProfile $ExpectedProfile)) {
        throw "TASK019_STORE_PROFILE_LIVE_GATE_REJECTED_$ExpectedProfile"
    }
}

function Invoke-LiveTest {
    param(
        [Parameter(Mandatory = $true)][string]$Cargo,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$Phase
    )

    $testOutput = @()
    $liveSuites = if ($MemoryOnly -and $Phase -eq 'restart') {
        @([pscustomobject]@{ Name = 'memory'; Package = 'lattice-postgres-codebase-memory' })
    }
    else {
        @(
            [pscustomobject]@{ Name = 'store'; Package = 'lattice-postgres-store' },
            [pscustomobject]@{ Name = 'memory'; Package = 'lattice-postgres-codebase-memory' }
        )
    }
    foreach ($suite in $liveSuites) {
        $suitePhase = if ($MemoryOnly -and $suite.Name -eq 'store') {
            'memory_setup'
        }
        else {
            $Phase
        }
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PHASE', $suitePhase, 'Process')
        $stdoutPath = Join-Path $clusterRoot ".cargo-$Phase-$($suite.Name)-stdout.log"
        $stderrPath = Join-Path $clusterRoot ".cargo-$Phase-$($suite.Name)-stderr.log"
        $process = $null
        $testExitCode = $null
        $suiteOutput = @()
        Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
        try {
            $process = Start-Process -FilePath $Cargo -ArgumentList @(
                'test',
                '-p', [string]$suite.Package,
                '--test', 'postgres_live',
                '--locked',
                '--',
                '--nocapture',
                '--test-threads=1'
            ) -WorkingDirectory $RepositoryRoot -RedirectStandardOutput $stdoutPath `
                -RedirectStandardError $stderrPath -WindowStyle Hidden -PassThru
            $null = $process.Handle
            $process.WaitForExit()
            $testExitCode = $process.ExitCode
            if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) {
                $suiteOutput += @(Get-Content -LiteralPath $stdoutPath -Encoding utf8)
            }
            if (Test-Path -LiteralPath $stderrPath -PathType Leaf) {
                $suiteOutput += @(Get-Content -LiteralPath $stderrPath -Encoding utf8)
            }
        }
        finally {
            if ($null -ne $process) {
                $process.Dispose()
            }
            foreach ($path in @($stdoutPath, $stderrPath)) {
                Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
                if (Test-Path -LiteralPath $path) {
                    throw 'TASK019_CARGO_OUTPUT_DELETE_FAILED'
                }
            }
        }
        if ($testExitCode -ne 0) {
            $safeTokens = @(
                $suiteOutput | ForEach-Object {
                    foreach ($match in [regex]::Matches(
                        [string]$_,
                        '(?<![A-Z0-9_])(?:TASK019|STORE|POSTGRES_TASK_LEDGER|MEMORY|OPENCLAW)_[A-Z0-9_]{1,63}(?![A-Z0-9_])'
                    )) {
                        $match.Value
                    }
                }
            )
            $safeTokens = @($safeTokens | Sort-Object -Unique)
            $safeSummary = if ($safeTokens.Count -eq 0) {
                'No allowlisted static diagnostic was emitted.'
            }
            else {
                $safeTokens -join ' | '
            }
            throw "$($suite.Name) postgres_live $Phase phase failed with exit code $testExitCode. Allowlisted diagnostics: $safeSummary"
        }
        $testOutput += $suiteOutput
        $storeProfile = Get-StoreProfileForLiveSuitePhase -Phase $Phase -SuiteName $suite.Name
        if ($null -ne $storeProfile) {
            Invoke-StoreProfileLiveGate `
                -Cargo $Cargo `
                -RepositoryRoot $RepositoryRoot `
                -ExpectedProfile $storeProfile `
                -Port $port `
                -Password $oneTimePassword `
                -RunId $runId
        }
    }
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PHASE', $Phase, 'Process')
    return ,$testOutput
}

function Get-RestartEvidence {
    param([Parameter(Mandatory = $true)][object[]]$TestOutput)

    $databaseId = $null
    $manifestHash = $null
    foreach ($item in $TestOutput) {
        $line = [string]$item
        # libtest may prefix the first uncaptured test line with `test <name> ... `.
        if ($line -match '(?:^|\s)TASK019_EVIDENCE database_uuid=([0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}) manifest_sha256=([0-9a-f]{64})$') {
            $databaseId = $Matches[1]
            $manifestHash = $Matches[2]
        }
    }

    if ($null -eq $databaseId -or $null -eq $manifestHash) {
        throw 'postgres_live initial phase did not emit the exact safe restart UUID/hash evidence.'
    }
    return [pscustomobject]@{
        DatabaseId = $databaseId
        ManifestHash = $manifestHash
    }
}

function Test-ClusterStopped {
    param(
        [Parameter(Mandatory = $true)][string]$PgCtl,
        [Parameter(Mandatory = $true)][string]$DataDirectory
    )

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $null = & $PgCtl '-D' $DataDirectory 'status' 2>&1
        $statusExitCode = [int]$LASTEXITCODE
        return ($statusExitCode -eq 3)
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
}

function Stop-TestCluster {
    param(
        [Parameter(Mandatory = $true)][string]$PgCtl,
        [Parameter(Mandatory = $true)][string]$DataDirectory
    )

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $null = & $PgCtl '-D' $DataDirectory '-m' 'fast' '-w' '-t' '30' 'stop' 2>&1
        $stopExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if ($stopExitCode -ne 0) {
        return (Test-ClusterStopped -PgCtl $PgCtl -DataDirectory $DataDirectory)
    }
    return (Test-ClusterStopped -PgCtl $PgCtl -DataDirectory $DataDirectory)
}

# Complexity justification: pg_ctl and tunnel-client create descendants whose lifetime can
# outlive a forcibly terminated harness, so the harness finally block is not sufficient.
# A detached tunnel runtime was rejected because it cannot preserve the same disposable
# PostgreSQL truth across the single authorized reconnect; a Rust async worker was rejected
# because it would add a protocol/state-machine change. This narrow watchdog owns cleanup
# only for the exact fresh cluster marker and exact pinned tunnel-client process tree.
function Start-Task038TunnelWatchdog {
    param(
        [Parameter(Mandatory = $true)][string]$PowerShell,
        [Parameter(Mandatory = $true)][string]$TunnelClient,
        [Parameter(Mandatory = $true)][string]$PgCtl,
        [Parameter(Mandatory = $true)][string]$ClusterRoot,
        [Parameter(Mandatory = $true)][string]$DataDirectory,
        [Parameter(Mandatory = $true)][string]$MarkerPath,
        [Parameter(Mandatory = $true)][string]$RepositoryTarget,
        [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{32}$')][string]$RunId,
        [Parameter(Mandatory = $true)][int]$ParentProcessId,
        [Parameter(Mandatory = $true)][AllowNull()]$ContainmentSnapshot
    )

    Assert-LatticeWindowsNativeContainmentSnapshot `
        -Snapshot $ContainmentSnapshot `
        -FailureCode 'TASK038_TUNNEL_WATCHDOG_CONTAINMENT_REJECTED'
    $source = $nativeIdentityHelperSource + [Environment]::NewLine + @'
$ErrorActionPreference = 'SilentlyContinue'
$parentPid = [int][Environment]::GetEnvironmentVariable('TASK038_WATCHDOG_PARENT_PID', 'Process')
$tunnelClient = [IO.Path]::GetFullPath([Environment]::GetEnvironmentVariable('TASK038_WATCHDOG_TUNNEL_CLIENT', 'Process'))
$pgCtl = [Environment]::GetEnvironmentVariable('TASK038_WATCHDOG_PG_CTL', 'Process')
$clusterRoot = [IO.Path]::GetFullPath([Environment]::GetEnvironmentVariable('TASK038_WATCHDOG_CLUSTER_ROOT', 'Process'))
$dataDirectory = [IO.Path]::GetFullPath([Environment]::GetEnvironmentVariable('TASK038_WATCHDOG_DATA', 'Process'))
$markerPath = [IO.Path]::GetFullPath([Environment]::GetEnvironmentVariable('TASK038_WATCHDOG_MARKER', 'Process'))
$repositoryTarget = [IO.Path]::GetFullPath([Environment]::GetEnvironmentVariable('TASK038_WATCHDOG_REPOSITORY_TARGET', 'Process'))
$runId = [Environment]::GetEnvironmentVariable('TASK038_WATCHDOG_RUN_ID', 'Process')
$containmentBase64 = [Environment]::GetEnvironmentVariable('TASK038_WATCHDOG_CONTAINMENT_B64', 'Process')
try {
    $containment = [Text.UTF8Encoding]::new($false, $true).GetString(
        [Convert]::FromBase64String($containmentBase64)
    ) | ConvertFrom-Json
    Assert-LatticeWindowsNativeContainmentSnapshot `
        -Snapshot $containment `
        -FailureCode 'TASK038_TUNNEL_WATCHDOG_CONTAINMENT_REJECTED'
}
catch { exit 0 }
$parentStartTimeUtcTicks = [long][Environment]::GetEnvironmentVariable('TASK038_WATCHDOG_PARENT_START_UTC_TICKS', 'Process')
while ($true) {
    $parent = Get-Process -Id $parentPid -ErrorAction SilentlyContinue
    if ($null -eq $parent) { break }
    try {
        if ($parent.StartTime.ToUniversalTime().Ticks -ne $parentStartTimeUtcTicks) { break }
    }
    catch { break }
    Start-Sleep -Seconds 2
}
if (-not (Test-LatticeWindowsNativeContainmentSnapshot -Snapshot $containment)) { exit 0 }
if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) { exit 0 }
try { $marker = Get-Content -LiteralPath $markerPath -Raw -Encoding UTF8 | ConvertFrom-Json } catch { exit 0 }
$expectedParent = [IO.Path]::GetFullPath((Join-Path $repositoryTarget 'task019-postgres'))
if (
    [string]$marker.kind -cne 'LATTICE_TASK019_DISPOSABLE_POSTGRES_V1' -or
    [string]$marker.run_id -cne $runId -or
    -not [string]::Equals([IO.Path]::GetFullPath([string]$marker.root), $clusterRoot, [StringComparison]::OrdinalIgnoreCase) -or
    -not [string]::Equals([IO.Path]::GetFullPath([string]$marker.repository_target), $repositoryTarget, [StringComparison]::OrdinalIgnoreCase) -or
    -not [string]::Equals([IO.Path]::GetFullPath((Split-Path -Parent $clusterRoot)), $expectedParent, [StringComparison]::OrdinalIgnoreCase)
) { exit 0 }
function Test-WatchdogNoReparseTree {
    param([Parameter(Mandatory = $true)][string]$Root)
    $rootItem = Get-Item -LiteralPath $Root -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $rootItem -or
        -not $rootItem.PSIsContainer -or
        ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) { return $false }
    $pending = [Collections.Generic.Queue[IO.DirectoryInfo]]::new()
    $pending.Enqueue($rootItem)
    try {
        while ($pending.Count -gt 0) {
            foreach ($item in $pending.Dequeue().EnumerateFileSystemInfos()) {
                if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { return $false }
                if ($item -is [IO.DirectoryInfo]) { $pending.Enqueue($item) }
            }
        }
    }
    catch { return $false }
    return $true
}
$pidRoot = Join-Path $clusterRoot 'task038-tunnel-pids'
$allTreesStopped = $true
# The managed launcher creates the tunnel root suspended, assigns it to a
# KILL_ON_JOB_CLOSE Job Object, and only then resumes it. Give Windows a bounded
# interval to close that parent-owned job before taking the cleanup snapshot.
Start-Sleep -Seconds 2
$processRows = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue)
$candidatePids = [Collections.Generic.HashSet[int]]::new()
if ($processRows.Count -lt 1) {
    $allTreesStopped = $false
}
if (Test-Path -LiteralPath $pidRoot -PathType Container) {
    $pidFiles = @(Get-ChildItem -LiteralPath $pidRoot -Filter 'task038-tunnel-session-*.pid' -File -Force -ErrorAction SilentlyContinue)
    foreach ($pidFile in $pidFiles) {
        $tunnelPid = 0
        $pidText = ''
        try { $pidText = [IO.File]::ReadAllText($pidFile.FullName).Trim() } catch {
            $allTreesStopped = $false
            continue
        }
        if (-not [int]::TryParse($pidText, [ref]$tunnelPid) -or $tunnelPid -lt 1) {
            $allTreesStopped = $false
            continue
        }
        [void]$candidatePids.Add($tunnelPid)
    }
}
else {
    $allTreesStopped = $false
}
foreach ($processRow in $processRows) {
    if (
        [int]$processRow.ParentProcessId -eq $parentPid -and
        -not [string]::IsNullOrWhiteSpace([string]$processRow.ExecutablePath) -and
        [string]::Equals([IO.Path]::GetFullPath([string]$processRow.ExecutablePath), $tunnelClient, [StringComparison]::OrdinalIgnoreCase)
    ) {
        [void]$candidatePids.Add([int]$processRow.ProcessId)
    }
}
# With a valid process snapshot, zero candidates is a proved NOT_STARTED or
# already-STOPPED session. Any root created before Job assignment still retains
# the dead harness PID as ParentProcessId and is added above by exact executable
# path; once assigned, parent death closes the Job and reaps the full tree.
foreach ($tunnelPid in $candidatePids) {
        $rootProcess = @($processRows | Where-Object { [int]$_.ProcessId -eq $tunnelPid })
        if ($rootProcess.Count -eq 0) {
            if (@($processRows | Where-Object { [int]$_.ParentProcessId -eq $tunnelPid }).Count -gt 0) {
                $allTreesStopped = $false
            }
            continue
        }
        if (
            $rootProcess.Count -ne 1 -or
            [string]::IsNullOrWhiteSpace([string]$rootProcess[0].ExecutablePath) -or
            -not [string]::Equals([IO.Path]::GetFullPath([string]$rootProcess[0].ExecutablePath), $tunnelClient, [StringComparison]::OrdinalIgnoreCase)
        ) {
            $allTreesStopped = $false
            continue
        }
        $descendantIds = [Collections.Generic.List[int]]::new()
        $frontier = @($tunnelPid)
        while ($frontier.Count -gt 0) {
            $next = [Collections.Generic.List[int]]::new()
            foreach ($processRow in $processRows) {
                if (
                    [int]$processRow.ParentProcessId -in $frontier -and
                    -not $descendantIds.Contains([int]$processRow.ProcessId)
                ) {
                    $descendantIds.Add([int]$processRow.ProcessId)
                    $next.Add([int]$processRow.ProcessId)
                }
            }
            $frontier = @($next)
        }
        Stop-Process -Id $tunnelPid -Force -ErrorAction SilentlyContinue
        foreach ($descendantId in $descendantIds) {
            Stop-Process -Id $descendantId -Force -ErrorAction SilentlyContinue
        }
        $remainingIds = @($tunnelPid) + @($descendantIds)
        $stopDeadline = [DateTime]::UtcNow.AddSeconds(15)
        do {
            $remaining = @($remainingIds | Where-Object {
                $null -ne (Get-Process -Id $_ -ErrorAction SilentlyContinue)
            })
            if ($remaining.Count -eq 0) { break }
            Start-Sleep -Milliseconds 100
        } while ([DateTime]::UtcNow -lt $stopDeadline)
        if ($remaining.Count -ne 0) {
            $allTreesStopped = $false
        }
}
if ($allTreesStopped) {
    $afterRows = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue)
    if ($afterRows.Count -lt 1) {
        $allTreesStopped = $false
    }
    else {
        foreach ($processRow in $afterRows) {
            if (
                [int]$processRow.ParentProcessId -eq $parentPid -and
                -not [string]::IsNullOrWhiteSpace([string]$processRow.ExecutablePath) -and
                [string]::Equals([IO.Path]::GetFullPath([string]$processRow.ExecutablePath), $tunnelClient, [StringComparison]::OrdinalIgnoreCase)
            ) {
                $allTreesStopped = $false
            }
        }
    }
}
$clusterStopped = -not (Test-Path -LiteralPath $dataDirectory -PathType Container)
if ($allTreesStopped -and -not $clusterStopped) {
    & $pgCtl '-D' $dataDirectory '-m' 'fast' '-w' '-t' '30' 'stop' *> $null
    & $pgCtl '-D' $dataDirectory 'status' *> $null
    $clusterStopped = ($LASTEXITCODE -eq 3)
}
if ($allTreesStopped -and -not (Test-LatticeWindowsNativeContainmentSnapshot -Snapshot $containment)) {
    $allTreesStopped = $false
}
$codexHome = Join-Path $clusterRoot 'task038-tunnel-codex-home'
if (Test-Path -LiteralPath $codexHome) {
    $homeItem = Get-Item -LiteralPath $codexHome -Force -ErrorAction SilentlyContinue
    $homeMarker = Join-Path $codexHome '.lattice-codex-home-v1'
    $ownerMarker = Join-Path $codexHome '.lattice-task038-tunnel-owner-v1'
    $homeMarkerItem = Get-Item -LiteralPath $homeMarker -Force -ErrorAction SilentlyContinue
    $ownerMarkerItem = Get-Item -LiteralPath $ownerMarker -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $homeItem -or -not $homeItem.PSIsContainer -or
        ($homeItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        $null -eq $homeMarkerItem -or $homeMarkerItem.PSIsContainer -or
        ($homeMarkerItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        $null -eq $ownerMarkerItem -or $ownerMarkerItem.PSIsContainer -or
        ($ownerMarkerItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        [IO.File]::ReadAllText($homeMarker, [Text.Encoding]::UTF8) -cne "lattice.codex-home.v1`n" -or
        [IO.File]::ReadAllText($ownerMarker, [Text.Encoding]::UTF8) -cnotmatch
            '^lattice\.task038-tunnel-home\.v1:[0-9a-f]{32}\r?\n$'
    ) {
        $allTreesStopped = $false
    }
    else {
        $authPath = Join-Path $codexHome 'auth.json'
        if (Test-Path -LiteralPath $authPath) {
            $authItem = Get-Item -LiteralPath $authPath -Force -ErrorAction SilentlyContinue
            if (
                $null -eq $authItem -or $authItem.PSIsContainer -or
                ($authItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
            ) {
                $allTreesStopped = $false
            }
            else {
                [IO.File]::Delete($authPath)
                if (Test-Path -LiteralPath $authPath) { $allTreesStopped = $false }
            }
        }
    }
}
$tunnelRuntimeRoot = Join-Path $clusterRoot 'task038-tunnel-runtime'
if (
    $allTreesStopped -and
    (Test-LatticeWindowsNativeContainmentSnapshot -Snapshot $containment) -and
    (Test-Path -LiteralPath $tunnelRuntimeRoot -PathType Container) -and
    (Test-WatchdogNoReparseTree -Root $tunnelRuntimeRoot)
) {
    Remove-Item -LiteralPath $tunnelRuntimeRoot -Recurse -Force -ErrorAction SilentlyContinue
}
if (
    $allTreesStopped -and $clusterStopped -and
    (Test-LatticeWindowsNativeContainmentSnapshot -Snapshot $containment) -and
    (Test-Path -LiteralPath $clusterRoot -PathType Container) -and
    (Test-WatchdogNoReparseTree -Root $clusterRoot)
) {
    Remove-Item -LiteralPath $clusterRoot -Recurse -Force -ErrorAction SilentlyContinue
}
'@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($source))
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $PowerShell
    $startInfo.Arguments = ('-NoProfile -NonInteractive -EncodedCommand ' + $encoded)
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    foreach ($name in @($startInfo.EnvironmentVariables.Keys)) {
        if (
            [string]$name -like 'LATTICE_*' -or
            [string]$name -eq 'CONTROL_PLANE_API_KEY' -or
            [string]$name -eq 'PGPASSWORD' -or
            [string]$name -match '(?i)(KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL|DSN)'
        ) {
            $startInfo.EnvironmentVariables.Remove([string]$name)
        }
    }
    $startInfo.EnvironmentVariables['TASK038_WATCHDOG_PARENT_PID'] = [string]$ParentProcessId
    $startInfo.EnvironmentVariables['TASK038_WATCHDOG_PARENT_START_UTC_TICKS'] = [string](Get-Process -Id $ParentProcessId -ErrorAction Stop).StartTime.ToUniversalTime().Ticks
    $startInfo.EnvironmentVariables['TASK038_WATCHDOG_TUNNEL_CLIENT'] = [IO.Path]::GetFullPath($TunnelClient)
    $startInfo.EnvironmentVariables['TASK038_WATCHDOG_PG_CTL'] = $PgCtl
    $startInfo.EnvironmentVariables['TASK038_WATCHDOG_CLUSTER_ROOT'] = $ClusterRoot
    $startInfo.EnvironmentVariables['TASK038_WATCHDOG_DATA'] = $DataDirectory
    $startInfo.EnvironmentVariables['TASK038_WATCHDOG_MARKER'] = $MarkerPath
    $startInfo.EnvironmentVariables['TASK038_WATCHDOG_REPOSITORY_TARGET'] = $RepositoryTarget
    $startInfo.EnvironmentVariables['TASK038_WATCHDOG_RUN_ID'] = $RunId
    $containmentJson = $ContainmentSnapshot | ConvertTo-Json -Depth 5 -Compress
    $startInfo.EnvironmentVariables['TASK038_WATCHDOG_CONTAINMENT_B64'] = [Convert]::ToBase64String(
        [Text.UTF8Encoding]::new($false).GetBytes($containmentJson)
    )
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        $process.Dispose()
        throw 'TASK038_TUNNEL_WATCHDOG_START_REJECTED'
    }
    return $process
}

function Remove-VerifiedSafeServerLog {
    param(
        [Parameter(Mandatory = $true)][string]$LogPath,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$OneTimePassword,
        [Parameter(Mandatory = $true)]$ContainmentSnapshot
    )

    Assert-LatticeWindowsNativeContainmentSnapshot `
        -Snapshot $ContainmentSnapshot `
        -FailureCode 'TASK019_CLEANUP_CONTAINMENT_REJECTED'
    if (-not (Test-Path -LiteralPath $LogPath -PathType Leaf)) {
        return
    }
    $content = Get-Content -LiteralPath $LogPath -Raw -Encoding utf8
    if ($null -eq $content) {
        $content = ''
    }
    $forbidden = @(
        $RepositoryRoot,
        $OneTimePassword,
        'intentional task019 rollback',
        'forbidden_table',
        'task019_ghost',
        'task019_unexpected',
        '11111111-1111-8111-8111-111111111111'
    )
    foreach ($value in $forbidden) {
        if (-not [string]::IsNullOrEmpty($value) -and
            $content.IndexOf($value, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
            try {
                [System.IO.File]::WriteAllBytes($LogPath, [byte[]]@())
            }
            catch {
                try {
                    Set-Content -LiteralPath $LogPath -Value $null -Encoding utf8 -NoNewline -ErrorAction Stop
                }
                catch {
                    throw 'TASK019_SERVER_LOG_SANITIZE_FAILED'
                }
            }
            if ((Get-Item -LiteralPath $LogPath -Force).Length -ne 0) {
                throw 'TASK019_SERVER_LOG_SANITIZE_FAILED'
            }
            try {
                Remove-Item -LiteralPath $LogPath -Force -ErrorAction Stop
            }
            catch {
                throw 'TASK019_SERVER_LOG_DELETE_FAILED'
            }
            if (Test-Path -LiteralPath $LogPath) {
                throw 'TASK019_SERVER_LOG_DELETE_FAILED'
            }
            throw 'TASK019_SERVER_LOG_REJECTED'
        }
    }
    Remove-Item -LiteralPath $LogPath -Force
    if (Test-Path -LiteralPath $LogPath) {
        throw 'TASK019_SERVER_LOG_DELETE_FAILED'
    }
}

function Remove-HarnessOutputFiles {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)]$ContainmentSnapshot
    )

    Assert-LatticeWindowsNativeContainmentSnapshot `
        -Snapshot $ContainmentSnapshot `
        -FailureCode 'TASK019_CLEANUP_CONTAINMENT_REJECTED'
    foreach ($outputPath in @(
        (Join-Path $Root '.native-stdout.log'),
        (Join-Path $Root '.native-stderr.log'),
        (Join-Path $Root '.cargo-initial-stdout.log'),
        (Join-Path $Root '.cargo-initial-stderr.log'),
        (Join-Path $Root '.cargo-restart-stdout.log'),
        (Join-Path $Root '.cargo-restart-stderr.log')
    )) {
        Remove-Item -LiteralPath $outputPath -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $outputPath) {
            throw 'TASK019_PROCESS_OUTPUT_DELETE_FAILED'
        }
    }
}

function Test-SafeCleanupTarget {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$ExpectedParent,
        [Parameter(Mandatory = $true)][string]$RepositoryTarget,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)]$ContainmentSnapshot
    )

    if ($RunId -notmatch '^[0-9a-f]{32}$') {
        return $false
    }

    $canonicalRoot = Get-CanonicalPath -Path $Root
    $canonicalParent = Get-CanonicalPath -Path $ExpectedParent
    $canonicalRepositoryTarget = Get-CanonicalPath -Path $RepositoryTarget
    $expectedRoot = Get-CanonicalPath -Path (Join-Path $canonicalParent $RunId)
    if (-not (Test-ExactPath -Actual $canonicalRoot -Expected $expectedRoot)) {
        return $false
    }
    if (-not (Test-ExactPath -Actual (Split-Path -Parent $canonicalRoot) -Expected $canonicalParent)) {
        return $false
    }

    $targetPrefix = $canonicalRepositoryTarget + [System.IO.Path]::DirectorySeparatorChar
    if (-not $canonicalRoot.StartsWith($targetPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }

    $rootItem = Get-Item -LiteralPath $canonicalRoot -Force -ErrorAction SilentlyContinue
    if ($null -eq $rootItem -or ($rootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
        return $false
    }

    $parentItem = Get-Item -LiteralPath $canonicalParent -Force -ErrorAction SilentlyContinue
    $targetItem = Get-Item -LiteralPath $canonicalRepositoryTarget -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $parentItem -or
        $null -eq $targetItem -or
        ($parentItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
        ($targetItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)
    ) {
        return $false
    }

    $markerPath = Join-Path $canonicalRoot $markerName
    if ($null -eq $ContainmentSnapshot) { return $false }
    if (
        -not (Test-ExactPath -Actual ([string]$ContainmentSnapshot.parent_path) -Expected $canonicalParent) -or
        -not (Test-ExactPath -Actual ([string]$ContainmentSnapshot.root_path) -Expected $canonicalRoot) -or
        -not (Test-ExactPath -Actual ([string]$ContainmentSnapshot.marker_path) -Expected $markerPath) -or
        -not (Test-LatticeWindowsNativeContainmentSnapshot -Snapshot $ContainmentSnapshot)
    ) {
        return $false
    }
    $markerItem = Get-Item -LiteralPath $markerPath -Force -ErrorAction SilentlyContinue
    if ($null -eq $markerItem -or ($markerItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
        return $false
    }

    try {
        $marker = Get-Content -LiteralPath $markerPath -Raw -Encoding utf8 | ConvertFrom-Json
    }
    catch {
        return $false
    }

    $requiredMarkerProperties = @('kind', 'run_id', 'root', 'parent', 'repository_target')
    foreach ($propertyName in $requiredMarkerProperties) {
        if ($propertyName -notin $marker.PSObject.Properties.Name) {
            return $false
        }
    }

    return (
        [string]$marker.kind -eq 'LATTICE_TASK019_DISPOSABLE_POSTGRES_V1' -and
        [string]$marker.run_id -eq $RunId -and
        (Test-ExactPath -Actual ([string]$marker.root) -Expected $canonicalRoot) -and
        (Test-ExactPath -Actual ([string]$marker.parent) -Expected $canonicalParent) -and
        (Test-ExactPath -Actual ([string]$marker.repository_target) -Expected $canonicalRepositoryTarget) -and
        (Test-NoReparseTree -Root $canonicalRoot)
    )
}

$repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
$repositoryTarget = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'target')
$clusterParent = Get-CanonicalPath -Path (Join-Path $repositoryTarget 'task019-postgres')
$runId = [Guid]::NewGuid().ToString('N')
$clusterRoot = Get-CanonicalPath -Path (Join-Path $clusterParent $runId)
$dataDirectory = Join-Path $clusterRoot 'data'
$passwordFile = Join-Path $clusterRoot '.initdb-password'
$serverLog = Join-Path $clusterRoot 'postgres.log'
$port = Get-UnreservedLoopbackPort
$oneTimePassword = $null
$clusterStarted = $false
$harnessCompleted = $false
$installedBefore = $null
$installedAfter = $null
$originalEnvironment = @{}
$deliveryHookPath = $null
$fullChainHookPath = $null
$task038HookPath = $null
$task038TunnelHookPath = $null
$task038TunnelWatchdog = $null
$cleanupContainment = $null

$selectedHookCount = @(
    $RunLatticeDeliveryHook,
    $RunFullChainAcceptanceHook,
    $RunTask038AcceptanceHook,
    $RunTask038TunnelHook
) |
    Where-Object { [bool]$_ }
if (@($selectedHookCount).Count -gt 1) {
    throw 'TASK019_HOOK_MODE_REJECTED'
}
if ($RunLatticeDeliveryHook) {
    $deliveryHookPath = Get-LatticeDeliveryHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
}
if ($RunFullChainAcceptanceHook) {
    $fullChainHookPath = Get-LatticeFullChainAcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
}
if ($RunTask038AcceptanceHook -or $RunTask038TunnelHook) {
    if ([string]::IsNullOrWhiteSpace($Task038OfficialCodexExecutable) -or [string]::IsNullOrWhiteSpace($Task038CodexAuthHome)) {
        throw 'TASK038_ACCEPTANCE_INPUT_REJECTED'
    }
    if (
        $RunTask038TunnelHook -and
        ([string]::IsNullOrWhiteSpace($Task038TunnelClientExecutable) -or
         [string]::IsNullOrWhiteSpace($Task038TunnelProfileDirectory) -or
         [string]::IsNullOrWhiteSpace($Task038EvidenceDirectory) -or
         [string]::IsNullOrWhiteSpace($Task038CanonicalAcceptanceReceipt) -or
         [string]::IsNullOrWhiteSpace($Task038CanonicalAcceptanceReceiptSha256))
    ) {
        throw 'TASK038_TUNNEL_RUNTIME_INPUT_REJECTED'
    }
    if ($RunTask038TunnelHook) {
        $task038TunnelHookPath = Get-LatticeTask038TunnelHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
    }
    else {
        $task038HookPath = Get-LatticeTask038AcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
    }
}

Invoke-HarnessSelfTest
Assert-NoReparseAncestor -Path $clusterRoot -Boundary $repositoryRoot

foreach ($name in $environmentNames) {
    $originalEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

foreach ($executable in $requiredExecutables) {
    $path = Join-Path $postgresBin $executable
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required PostgreSQL 17.10 executable is missing: $executable"
    }
}

$initdb = Join-Path $postgresBin 'initdb.exe'
$pgCtl = Join-Path $postgresBin 'pg_ctl.exe'
$pgIsReady = Join-Path $postgresBin 'pg_isready.exe'
$postgres = Join-Path $postgresBin 'postgres.exe'
$cargoCommand = Get-Command 'cargo.exe' -ErrorAction Stop

$previousErrorActionPreference = $ErrorActionPreference
try {
    $ErrorActionPreference = 'Continue'
    $versionOutput = @(& $postgres '--version' 2>&1)
    $versionExitCode = $LASTEXITCODE
}
finally {
    $ErrorActionPreference = $previousErrorActionPreference
}
if ($versionExitCode -ne 0 -or (($versionOutput -join "`n") -notmatch "postgres \(PostgreSQL\) $([regex]::Escape($expectedPostgresVersion))(?:\s|$)")) {
    throw "The harness requires PostgreSQL $expectedPostgresVersion exactly."
}

$installedBefore = Get-InstalledPostgresSnapshot -PgIsReady $pgIsReady

try {
    New-Item -ItemType Directory -Path $clusterRoot -Force:$false | Out-Null
    Assert-NoReparseAncestor -Path $clusterRoot -Boundary $repositoryRoot
    $marker = [ordered]@{
        kind = 'LATTICE_TASK019_DISPOSABLE_POSTGRES_V1'
        run_id = $runId
        root = $clusterRoot
        parent = $clusterParent
        repository_target = $repositoryTarget
        postgres_version = $expectedPostgresVersion
    }
    $marker | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $clusterRoot $markerName) -Encoding utf8
    $cleanupContainment = New-LatticeWindowsNativeContainmentSnapshot `
        -ParentPath $clusterParent `
        -RootPath $clusterRoot `
        -MarkerPath (Join-Path $clusterRoot $markerName)

    $oneTimePassword = New-OneTimePassword
    try {
        Set-Content -LiteralPath $passwordFile -Value $oneTimePassword -Encoding ascii -NoNewline
        $null = Invoke-NativeChecked -Executable $initdb -Arguments @(
            '--pgdata', $dataDirectory,
            '--encoding', 'UTF8',
            '--locale', 'C',
            '--data-checksums',
            '--username', $harnessUser,
            '--pwfile', $passwordFile,
            '--auth-host', 'scram-sha-256',
            '--auth-local', 'scram-sha-256'
        ) -Operation 'initdb'
    }
    finally {
        if (Test-Path -LiteralPath $passwordFile) {
            Remove-Item -LiteralPath $passwordFile -Force
        }
    }

    @(
        "listen_addresses = '127.0.0.1'"
        "port = $port"
        'ssl = off'
        'fsync = on'
        'synchronous_commit = on'
        'full_page_writes = on'
        'max_prepared_transactions = 0'
        'password_encryption = scram-sha-256'
        'logging_collector = off'
        "log_min_messages = 'panic'"
        "log_min_error_statement = 'panic'"
        'log_parameter_max_length_on_error = 0'
        "log_error_verbosity = 'terse'"
        "log_connections = off"
        "log_disconnections = off"
        "log_statement = 'none'"
    ) | Add-Content -LiteralPath (Join-Path $dataDirectory 'postgresql.conf') -Encoding ascii

    $clusterStarted = $true
    $null = Invoke-NativeChecked -Executable $pgCtl -Arguments @(
        '-D', $dataDirectory,
        '-l', $serverLog,
        '-w',
        '-t', '30',
        'start'
    ) -Operation 'PostgreSQL test-cluster start'
    Set-HarnessEnvironment -Phase 'initial' -HostName '127.0.0.1' -Port $port -Password $oneTimePassword -RunId $runId
    $initialOutput = Invoke-LiveTest -Cargo $cargoCommand.Source -RepositoryRoot $repositoryRoot -Phase 'initial'
    $restartEvidence = Get-RestartEvidence -TestOutput $initialOutput

    if (-not (Stop-TestCluster -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
        throw 'Could not prove the disposable PostgreSQL cluster stopped after the initial phase.'
    }
    $clusterStarted = $false
    Remove-HarnessOutputFiles -Root $clusterRoot -ContainmentSnapshot $cleanupContainment
    Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword -ContainmentSnapshot $cleanupContainment

    $clusterStarted = $true
    $null = Invoke-NativeChecked -Executable $pgCtl -Arguments @(
        '-D', $dataDirectory,
        '-l', $serverLog,
        '-w',
        '-t', '30',
        'start'
    ) -Operation 'PostgreSQL test-cluster restart'
    Set-HarnessEnvironment -Phase 'restart' -HostName '127.0.0.1' -Port $port -Password $oneTimePassword -RunId $runId
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_UUID', $restartEvidence.DatabaseId, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_MANIFEST', $restartEvidence.ManifestHash, 'Process')
    $null = Invoke-LiveTest -Cargo $cargoCommand.Source -RepositoryRoot $repositoryRoot -Phase 'restart'

    if ($RunLatticeDeliveryHook) {
        $deliveryHookPath = Get-LatticeDeliveryHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
        & $deliveryHookPath -InternalPhase 'DeliveryRun'

        if (-not (Stop-TestCluster -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
            throw 'Could not prove the disposable PostgreSQL cluster stopped after the delivery-run phase.'
        }
        $clusterStarted = $false
        Remove-HarnessOutputFiles -Root $clusterRoot -ContainmentSnapshot $cleanupContainment
        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword -ContainmentSnapshot $cleanupContainment

        $clusterStarted = $true
        $null = Invoke-NativeChecked -Executable $pgCtl -Arguments @(
            '-D', $dataDirectory,
            '-l', $serverLog,
            '-w',
            '-t', '30',
            'start'
        ) -Operation 'PostgreSQL test-cluster delivery restart'
        Set-HarnessEnvironment -Phase 'restart' -HostName '127.0.0.1' -Port $port -Password $oneTimePassword -RunId $runId
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_UUID', $restartEvidence.DatabaseId, 'Process')
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_MANIFEST', $restartEvidence.ManifestHash, 'Process')

        $deliveryHookPath = Get-LatticeDeliveryHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
        & $deliveryHookPath -InternalPhase 'DeliveryStatus'
    }
    elseif ($RunFullChainAcceptanceHook) {
        $fullChainHookPath = Get-LatticeFullChainAcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
        & $fullChainHookPath -InternalPhase 'FullChainPreStatus'

        if (-not (Stop-TestCluster -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
            throw 'Could not prove the disposable PostgreSQL cluster stopped after the full-chain pre-status phase.'
        }
        $clusterStarted = $false
        Remove-HarnessOutputFiles -Root $clusterRoot -ContainmentSnapshot $cleanupContainment
        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword -ContainmentSnapshot $cleanupContainment

        $clusterStarted = $true
        $null = Invoke-NativeChecked -Executable $pgCtl -Arguments @(
            '-D', $dataDirectory,
            '-l', $serverLog,
            '-w',
            '-t', '30',
            'start'
        ) -Operation 'PostgreSQL test-cluster full-chain run restart'
        Set-HarnessEnvironment -Phase 'restart' -HostName '127.0.0.1' -Port $port -Password $oneTimePassword -RunId $runId
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_UUID', $restartEvidence.DatabaseId, 'Process')
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_MANIFEST', $restartEvidence.ManifestHash, 'Process')

        $fullChainHookPath = Get-LatticeFullChainAcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
        & $fullChainHookPath -InternalPhase 'FullChainRun'

        if (-not (Stop-TestCluster -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
            throw 'Could not prove the disposable PostgreSQL cluster stopped after the full-chain run phase.'
        }
        $clusterStarted = $false
        Remove-HarnessOutputFiles -Root $clusterRoot -ContainmentSnapshot $cleanupContainment
        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword -ContainmentSnapshot $cleanupContainment

        $clusterStarted = $true
        $null = Invoke-NativeChecked -Executable $pgCtl -Arguments @(
            '-D', $dataDirectory,
            '-l', $serverLog,
            '-w',
            '-t', '30',
            'start'
        ) -Operation 'PostgreSQL test-cluster full-chain status restart'
        Set-HarnessEnvironment -Phase 'restart' -HostName '127.0.0.1' -Port $port -Password $oneTimePassword -RunId $runId
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_UUID', $restartEvidence.DatabaseId, 'Process')
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_MANIFEST', $restartEvidence.ManifestHash, 'Process')

        $fullChainHookPath = Get-LatticeFullChainAcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
        & $fullChainHookPath -InternalPhase 'FullChainStatus'
    }
    elseif ($RunTask038AcceptanceHook -or $RunTask038TunnelHook) {
        $task038HookPath = Get-LatticeTask038AcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
        $task038DatabaseName = 'lattice_task019_' + $runId.Substring(0, 8) + '_base'
        $encodedPassword = [Uri]::EscapeDataString($oneTimePassword)
        $task038Environment = [ordered]@{
            LATTICE_TASK038_POSTGRES_PASSWORD = $oneTimePassword
            LATTICE_WRITER_LEASE_MIGRATOR_URL = ('postgresql://lattice_migrator_login:{0}@127.0.0.1:{1}/{2}' -f $encodedPassword, $port, $task038DatabaseName)
            LATTICE_WRITER_LEASE_RUNTIME_URL = ('postgresql://lattice_runtime_login:{0}@127.0.0.1:{1}/{2}' -f $encodedPassword, $port, $task038DatabaseName)
            LATTICE_WRITER_LEASE_ADMIN_URL = ('postgresql://task019_harness:{0}@127.0.0.1:{1}/postgres' -f $encodedPassword, $port)
        }
        foreach ($entry in $task038Environment.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
        }
        if ($RunTask038TunnelHook) {
            $task038TunnelWatchdog = Start-Task038TunnelWatchdog `
                -PowerShell (Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)) 'System32\WindowsPowerShell\v1.0\powershell.exe') `
                -TunnelClient $Task038TunnelClientExecutable `
                -PgCtl $pgCtl `
                -ClusterRoot $clusterRoot `
                -DataDirectory $dataDirectory `
                -MarkerPath (Join-Path $clusterRoot $markerName) `
                -RepositoryTarget $repositoryTarget `
                -RunId $runId `
                -ParentProcessId $PID `
                -ContainmentSnapshot $cleanupContainment
            $task038TunnelHookPath = Get-LatticeTask038TunnelHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
            & $task038TunnelHookPath `
                -Mode ManagedRun `
                -TunnelClientExecutable $Task038TunnelClientExecutable `
                -ProfileDirectory $Task038TunnelProfileDirectory `
                -ProfileName $Task038TunnelProfileName `
                -OfficialCodexExecutable $Task038OfficialCodexExecutable `
                -CodexAuthHome $Task038CodexAuthHome `
                -PsqlExecutable (Join-Path $postgresBin 'psql.exe') `
                -PostgresDataDirectory $dataDirectory `
                -ManagedEvidenceDirectory $Task038EvidenceDirectory `
                -CanonicalAcceptanceReceipt $Task038CanonicalAcceptanceReceipt `
                -CanonicalAcceptanceReceiptSha256 $Task038CanonicalAcceptanceReceiptSha256
        }
        else {
            & $task038HookPath `
                -OfficialCodexExecutable $Task038OfficialCodexExecutable `
                -CodexAuthHome $Task038CodexAuthHome `
                -PostgresPort $port `
                -PostgresRunId $runId `
                -PsqlExecutable (Join-Path $postgresBin 'psql.exe') `
                -PostgresDataDirectory $dataDirectory
            Invoke-StoreProfileLiveGate `
                -Cargo $cargoCommand.Source `
                -RepositoryRoot $repositoryRoot `
                -ExpectedProfile 'V3_MEMORY_V2_WRITER_LEASE_V1' `
                -Port $port `
                -Password $oneTimePassword `
                -RunId $runId
        }
        foreach ($entry in $task038Environment.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, $originalEnvironment[[string]$entry.Key], 'Process')
        }
    }
    $harnessCompleted = $true
}
finally {
    try {
        foreach ($name in $environmentNames) {
            [Environment]::SetEnvironmentVariable($name, $originalEnvironment[$name], 'Process')
        }

        if (Test-Path -LiteralPath $passwordFile) {
            Remove-Item -LiteralPath $passwordFile -Force -ErrorAction SilentlyContinue
        }
        if (Test-Path -LiteralPath $clusterRoot -PathType Container) {
            Assert-LatticeWindowsNativeContainmentSnapshot `
                -Snapshot $cleanupContainment `
                -FailureCode 'TASK019_CLEANUP_CONTAINMENT_REJECTED'
            Remove-Task038DisposableCodexAuth `
                -ClusterRoot $clusterRoot `
                -RepositoryRoot $repositoryRoot
        }
        if ($clusterStarted) {
            if (-not (Stop-TestCluster -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
                throw "Disposable cluster could not be proved stopped; preserving $clusterRoot"
            }
            $clusterStarted = $false
        }
        elseif ((Test-Path -LiteralPath $dataDirectory) -and -not (Test-ClusterStopped -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
            throw "Disposable cluster status is not safely stopped; preserving $clusterRoot"
        }

        if (Test-Path -LiteralPath $clusterRoot -PathType Container) {
            Remove-HarnessOutputFiles -Root $clusterRoot -ContainmentSnapshot $cleanupContainment
            Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword -ContainmentSnapshot $cleanupContainment
        }
        $oneTimePassword = $null

        if (Test-Path -LiteralPath $clusterRoot) {
            $cleanupTargetIsExact = Test-SafeCleanupTarget `
                -Root $clusterRoot `
                -ExpectedParent $clusterParent `
                -RepositoryTarget $repositoryTarget `
                -RunId $runId `
                -ContainmentSnapshot $cleanupContainment
            if (-not $cleanupTargetIsExact) {
                throw "Disposable cluster cleanup gate did not pass; preserving $clusterRoot"
            }
            Remove-Item -LiteralPath $clusterRoot -Recurse -Force
            if (Test-Path -LiteralPath $clusterRoot) {
                throw "Disposable cluster cleanup could not be proved complete; preserving $clusterRoot"
            }
        }
    }
    finally {
        if ($null -ne $task038TunnelWatchdog) {
            try {
                if (-not $task038TunnelWatchdog.HasExited) {
                    $task038TunnelWatchdog.Kill()
                }
                if (-not $task038TunnelWatchdog.WaitForExit(5000)) {
                    throw 'TASK038_TUNNEL_WATCHDOG_NOT_REAPED'
                }
            }
            finally {
                $task038TunnelWatchdog.Dispose()
            }
        }
        $installedAfter = Get-InstalledPostgresSnapshot -PgIsReady $pgIsReady
        if (-not (Test-SameInstalledPostgresSnapshot -Before $installedBefore -After $installedAfter)) {
            throw 'Installed postgresql-x64-17 service or its read-only 127.0.0.1:5432 readiness snapshot changed during the harness.'
        }
    }
}

if (-not $harnessCompleted) {
    throw 'TASK-019 live phases did not complete.'
}
Write-Output 'TASK019_POSTGRES_HARNESS=PASS'
Write-Output "POSTGRES_VERSION=$expectedPostgresVersion"
Write-Output 'ENDPOINT=127.0.0.1:<ephemeral-non-5432>'
Write-Output 'PHASES=initial,restart'
