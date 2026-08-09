[CmdletBinding()]
param(
    [switch]$RunLatticeDeliveryHook,
    [switch]$RunFullChainAcceptanceHook,
    [switch]$RunTask038AcceptanceHook,
    [string]$Task038OfficialCodexExecutable,
    [string]$Task038CodexAuthHome,
    [switch]$MemoryOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

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

function Remove-VerifiedSafeServerLog {
    param(
        [Parameter(Mandatory = $true)][string]$LogPath,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$OneTimePassword
    )

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
    param([Parameter(Mandatory = $true)][string]$Root)

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
        [Parameter(Mandatory = $true)][string]$RunId
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
        (Test-ExactPath -Actual ([string]$marker.repository_target) -Expected $canonicalRepositoryTarget)
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

$selectedHookCount = @($RunLatticeDeliveryHook, $RunFullChainAcceptanceHook, $RunTask038AcceptanceHook) |
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
if ($RunTask038AcceptanceHook) {
    if ([string]::IsNullOrWhiteSpace($Task038OfficialCodexExecutable) -or [string]::IsNullOrWhiteSpace($Task038CodexAuthHome)) {
        throw 'TASK038_ACCEPTANCE_INPUT_REJECTED'
    }
    $task038HookPath = Get-LatticeTask038AcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
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
    Remove-HarnessOutputFiles -Root $clusterRoot
    Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword

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
        Remove-HarnessOutputFiles -Root $clusterRoot
        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword

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
        Remove-HarnessOutputFiles -Root $clusterRoot
        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword

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
        Remove-HarnessOutputFiles -Root $clusterRoot
        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword

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
    elseif ($RunTask038AcceptanceHook) {
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
        if ($clusterStarted) {
            if (-not (Stop-TestCluster -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
                throw "Disposable cluster could not be proved stopped; preserving $clusterRoot"
            }
            $clusterStarted = $false
        }
        elseif ((Test-Path -LiteralPath $dataDirectory) -and -not (Test-ClusterStopped -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
            throw "Disposable cluster status is not safely stopped; preserving $clusterRoot"
        }

        Remove-HarnessOutputFiles -Root $clusterRoot
        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword
        $oneTimePassword = $null

        if (Test-Path -LiteralPath $clusterRoot) {
            $cleanupTargetIsExact = Test-SafeCleanupTarget -Root $clusterRoot -ExpectedParent $clusterParent -RepositoryTarget $repositoryTarget -RunId $runId
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
