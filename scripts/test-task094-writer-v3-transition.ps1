[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern('^[0-9a-f]{32}$')]
    [string]$RunId,

    [Parameter(Mandatory)]
    [ValidateRange(1, 65535)]
    [int]$Port,

    [Parameter(Mandatory)]
    [string]$RepositoryRoot,

    [ValidateRange(60, 3600)]
    [int]$CargoTimeoutSeconds = 1200,

    [ValidateRange(10, 300)]
    [int]$PostgresProcessTimeoutSeconds = 90,

    [switch]$CleanupInterrupted,

    [switch]$InjectInitdbFailure,

    [switch]$InjectTeardownOwnershipFailure,

    [switch]$InjectTeardownStatusFailure
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$script:task094ProcessTreeCleanupProven = $true

if ($Port -in @(5432, 58743)) {
    throw 'TASK094_FORBIDDEN_PORT'
}

function Get-Task094ListenerPid {
    $pattern = '^\s*TCP\s+127\.0\.0\.1:' + $Port + '\s+\S+\s+LISTENING\s+(\d+)\s*$'
    @(
        & "$env:SystemRoot\System32\netstat.exe" -ano -p tcp |
            ForEach-Object {
                if ($_ -match $pattern) {
                    [int]$Matches[1]
                }
            }
    )
}

function Start-Task094Process {
    param(
        [Parameter(Mandatory)]
        [string]$FilePath,

        [Parameter(Mandatory)]
        [string[]]$ArgumentList,

        [Parameter(Mandatory)]
        [string]$WorkingDirectory,

        [hashtable]$Environment = @{}
    )

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    foreach ($argument in $ArgumentList) {
        $null = $startInfo.ArgumentList.Add($argument)
    }
    foreach ($entry in $Environment.GetEnumerator()) {
        $startInfo.Environment[$entry.Key] = [string]$entry.Value
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw 'TASK094_PROCESS_START_FAILED'
    }
    $process
}

function Wait-Task094OwnedProcess {
    param(
        [Parameter(Mandatory)]
        [Diagnostics.Process]$Process,

        [Parameter(Mandatory)]
        [int]$TimeoutSeconds,

        [Parameter(Mandatory)]
        [string]$TimeoutDiagnostic
    )

    if (-not $Process.WaitForExit($TimeoutSeconds * 1000)) {
        try {
            $Process.Kill($true)
        }
        catch {
            Write-Warning "$TimeoutDiagnostic process-tree termination reported: $($_.Exception.Message)"
        }
        try {
            $processTreeExited = $Process.WaitForExit(5000)
        }
        catch {
            $script:task094ProcessTreeCleanupProven = $false
            throw 'TASK094_PROCESS_TREE_EXIT_OBSERVATION_FAILED'
        }
        if (-not $processTreeExited) {
            $script:task094ProcessTreeCleanupProven = $false
            throw 'TASK094_PROCESS_TREE_SURVIVOR'
        }
        throw $TimeoutDiagnostic
    }
}

function Write-Task094RedactedFile {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    Get-Content -LiteralPath $Path -ErrorAction SilentlyContinue |
        ForEach-Object { ([string]$_).Replace($password, '[REDACTED]') } |
        Out-Host
}

$postgresBin = 'C:\Program Files\PostgreSQL\17\bin'
$initdb = Join-Path $postgresBin 'initdb.exe'
$pgCtl = Join-Path $postgresBin 'pg_ctl.exe'
$postgres = Join-Path $postgresBin 'postgres.exe'
foreach ($binary in @($initdb, $pgCtl, $postgres)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw 'TASK094_POSTGRES_BINARY_MISSING'
    }
}

$tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')
$runRoot = [IO.Path]::GetFullPath((Join-Path $tempRoot "lattice-task094-pg-$RunId"))
$expectedRoot = "$tempRoot\lattice-task094-pg-$RunId"
if ($runRoot -cne $expectedRoot -or [IO.Path]::GetFileName($runRoot) -cne "lattice-task094-pg-$RunId") {
    throw 'TASK094_RUN_ROOT_REJECTED'
}
if ($CleanupInterrupted) {
    if (-not (Test-Path -LiteralPath $runRoot -PathType Container)) {
        throw 'TASK094_INTERRUPTED_ROOT_MISSING'
    }
    $owner = Get-Content -LiteralPath (Join-Path $runRoot 'TASK094_OWNER.json') -Raw |
        ConvertFrom-Json
    if (
        $owner.owner -cne 'TASK-094' -or
        $owner.run_id -cne $RunId -or
        [int]$owner.port -ne $Port
    ) {
        throw 'TASK094_INTERRUPTED_MARKER_MISMATCH'
    }
    if (@(Get-Task094ListenerPid).Count -ne 0) {
        throw 'TASK094_INTERRUPTED_LISTENER_PRESENT'
    }
    if ([IO.Path]::GetFullPath([string]$owner.postgres_executable) -cne [IO.Path]::GetFullPath($postgres)) {
        throw 'TASK094_INTERRUPTED_POSTGRES_IDENTITY_MISMATCH'
    }
    $interruptedDataRoot = Join-Path $runRoot 'data'
    $interruptedPgVersion = Join-Path $interruptedDataRoot 'PG_VERSION'
    if (Test-Path -LiteralPath $interruptedPgVersion -PathType Leaf) {
        $statusProcess = Start-Task094Process -FilePath $pgCtl `
            -ArgumentList @('-D', $interruptedDataRoot, 'status') `
            -WorkingDirectory $runRoot
        Wait-Task094OwnedProcess -Process $statusProcess `
            -TimeoutSeconds $PostgresProcessTimeoutSeconds `
            -TimeoutDiagnostic 'TASK094_INTERRUPTED_STATUS_TIMEOUT'
        if ($statusProcess.ExitCode -ne 3) {
            throw 'TASK094_INTERRUPTED_SERVER_NOT_STOPPED'
        }
    }
    elseif (Test-Path -LiteralPath (Join-Path $interruptedDataRoot 'postmaster.pid')) {
        throw 'TASK094_INTERRUPTED_PARTIAL_CLUSTER_PID_PRESENT'
    }
    Remove-Item -LiteralPath $runRoot -Recurse -Force
    Write-Output "TASK094_INTERRUPTED_ROOT_REMOVED root_absent=$(-not (Test-Path -LiteralPath $runRoot))"
    exit 0
}
$repository = [IO.Path]::GetFullPath($RepositoryRoot)
if (-not (Test-Path -LiteralPath (Join-Path $repository 'Cargo.toml') -PathType Leaf)) {
    throw 'TASK094_REPOSITORY_REJECTED'
}
$approvedRustToolchain = '1.97.1'
$cargo = (Get-Command cargo.exe -CommandType Application -ErrorAction Stop)[0].Source
$cargoVersion = @(& $cargo "+$approvedRustToolchain" --version 2>&1)
if (
    $LASTEXITCODE -ne 0 -or
    $cargoVersion.Count -ne 1 -or
    [string]$cargoVersion[0] -cne 'cargo 1.97.1 (c980f4866 2026-06-30)'
) {
    throw 'TASK094_APPROVED_CARGO_TOOLCHAIN_UNAVAILABLE'
}
Write-Output "TASK094_RUST_TOOLCHAIN_OK toolchain=$approvedRustToolchain cargo_version=$($cargoVersion[0])"
if (Test-Path -LiteralPath $runRoot) {
    throw 'TASK094_RUN_ROOT_COLLISION'
}
if (@(Get-Task094ListenerPid).Count -ne 0) {
    throw 'TASK094_PORT_COLLISION'
}

$dataRoot = Join-Path $runRoot 'data'
$logPath = Join-Path $runRoot 'postgres.log'
$markerPath = Join-Path $runRoot 'TASK094_OWNER.json'
$passwordPath = Join-Path $runRoot '.initdb-password'
$cargoOutputPath = Join-Path $runRoot 'cargo-test.log'
$cargoErrorPath = Join-Path $runRoot 'cargo-test.err'
$catalogOutputPath = Join-Path $runRoot 'catalog-measurement.log'
$catalogErrorPath = Join-Path $runRoot 'catalog-measurement.err'
$ownedPostgresPid = $null
$ownedPostgresStartTicks = $null
$liveGateObserved = $false
$password = [Convert]::ToHexString(
    [Security.Cryptography.RandomNumberGenerator]::GetBytes(24)
).ToLowerInvariant()

try {
    New-Item -ItemType Directory -Path $runRoot | Out-Null
    [ordered]@{
        owner = 'TASK-094'
        run_id = $RunId
        port = $Port
        created_utc = [DateTime]::UtcNow.ToString('o')
        postgres_executable = $postgres
    } | ConvertTo-Json | Set-Content -LiteralPath $markerPath -Encoding utf8NoBOM
    Set-Content -LiteralPath $passwordPath -Value $password -Encoding ascii -NoNewline

    $initdbArguments = @(
        '-D', $dataRoot, '-U', 'task019_harness', '-A', 'scram-sha-256',
        "--pwfile=$passwordPath", '--encoding=UTF8', '--locale=C', '--data-checksums'
    )
    if ($InjectInitdbFailure) {
        $initdbArguments += '--task094-injected-invalid-option'
    }
    $initdbProcess = Start-Task094Process -FilePath $initdb `
        -ArgumentList $initdbArguments -WorkingDirectory $runRoot
    Wait-Task094OwnedProcess -Process $initdbProcess `
        -TimeoutSeconds $PostgresProcessTimeoutSeconds `
        -TimeoutDiagnostic 'TASK094_INITDB_TIMEOUT'
    if ($initdbProcess.ExitCode -ne 0) {
        throw 'TASK094_INITDB_FAILED'
    }
    Remove-Item -LiteralPath $passwordPath -Force

    $postgresOptions = "-p $Port -h 127.0.0.1 -c ssl=off -c fsync=on " +
        '-c synchronous_commit=on -c full_page_writes=on -c max_prepared_transactions=0 ' +
        '-c log_statement=none -c log_min_error_statement=PANIC ' +
        '-c log_parameter_max_length_on_error=0'
    $startProcess = Start-Task094Process -FilePath $pgCtl `
        -ArgumentList @('-D', $dataRoot, '-l', $logPath, '-o', $postgresOptions, 'start') `
        -WorkingDirectory $runRoot
    Wait-Task094OwnedProcess -Process $startProcess `
        -TimeoutSeconds $PostgresProcessTimeoutSeconds `
        -TimeoutDiagnostic 'TASK094_PG_START_TIMEOUT'
    if ($startProcess.ExitCode -ne 0) {
        throw 'TASK094_PG_START_FAILED'
    }

    $pidPath = Join-Path $dataRoot 'postmaster.pid'
    Write-Host 'TASK094_OWNERSHIP_CHECK_PID_ENTER'
    $postgresPid = [int]((Get-Content -LiteralPath $pidPath -TotalCount 1).Trim())
    $process = Get-Process -Id $postgresPid -ErrorAction Stop
    Write-Host 'TASK094_OWNERSHIP_CHECK_LISTENER_ENTER'
    $listenerPids = @(Get-Task094ListenerPid)
    Write-Host 'TASK094_OWNERSHIP_CHECK_MARKER_ENTER'
    $owner = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
    if (
        $listenerPids.Count -ne 1 -or
        $listenerPids[0] -ne $postgresPid -or
        [IO.Path]::GetFullPath($process.Path) -cne [IO.Path]::GetFullPath($postgres) -or
        $owner.owner -cne 'TASK-094' -or
        $owner.run_id -cne $RunId -or
        [int]$owner.port -ne $Port
    ) {
        throw 'TASK094_LIVE_OWNERSHIP_REJECTED'
    }
    $ownedPostgresPid = $postgresPid
    $ownedPostgresStartTicks = $process.StartTime.ToUniversalTime().Ticks
    Write-Output "TASK094_LIVE_OWNERSHIP_OK run_root=$runRoot port=$Port pid=$postgresPid"
    if ($InjectTeardownOwnershipFailure) {
        [IO.File]::WriteAllText(
            $cargoErrorPath,
            "TASK094_SECRET_SCRUB_PROBE=$password`n",
            [Text.UTF8Encoding]::new($false)
        )
        $ownedPostgresStartTicks += 1
        throw 'TASK094_INJECTED_TEARDOWN_OWNERSHIP_FAILURE'
    }
    if ($InjectTeardownStatusFailure) {
        throw 'TASK094_INJECTED_TEARDOWN_STATUS_FAILURE'
    }

    Write-Host 'TASK094_CARGO_TEST_ENTER'
    $cargoArguments = @(
        "+$approvedRustToolchain", 'test', '-p', 'lattice-runtime', '--test', 'task094_writer_v3_transition',
        '--locked', 'task094_writer_v3_transition_composition', '--',
        '--ignored', '--exact', '--nocapture'
    )
    $cargoProcess = Start-Process -FilePath $cargo -ArgumentList $cargoArguments `
        -WorkingDirectory $repository -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $cargoOutputPath -RedirectStandardError $cargoErrorPath `
        -Environment @{
            LATTICE_TASK019_LIVE = '1'
            LATTICE_TASK019_HOST = '127.0.0.1'
            LATTICE_TASK019_PORT = [string]$Port
            LATTICE_TASK019_PASSWORD = $password
            LATTICE_TASK019_RUN_ID = $RunId
            LATTICE_TASK019_PHASE = 'task094_transition'
        }
    try {
        Wait-Task094OwnedProcess -Process $cargoProcess `
            -TimeoutSeconds $CargoTimeoutSeconds `
            -TimeoutDiagnostic 'TASK094_CARGO_TEST_TIMEOUT'
    }
    finally {
        Write-Task094RedactedFile -Path $cargoOutputPath
        Write-Task094RedactedFile -Path $cargoErrorPath
    }
    if ($cargoProcess.ExitCode -ne 0) {
        throw 'TASK094_FOCUSED_LIVE_TEST_FAILED'
    }
    $cargoEvidence = @(
        Get-Content -LiteralPath $cargoOutputPath -ErrorAction Stop
        Get-Content -LiteralPath $cargoErrorPath -ErrorAction Stop
    )
    $transitionPattern = '^TASK094_TRANSITION_OK database_uuid=[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12} manifest_sha256=584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8$'
    $transitionMarkers = @($cargoEvidence | Where-Object { $_ -cmatch $transitionPattern })
    $exactResults = @(
        $cargoEvidence |
            Where-Object { $_ -cmatch '^test result: ok\. 1 passed; 0 failed; 0 ignored;' }
    )
    if ($transitionMarkers.Count -ne 1 -or $exactResults.Count -ne 1) {
        throw 'TASK094_LIVE_TEST_NOT_OBSERVED'
    }

    Write-Host 'TASK094_V7_CATALOG_MEASUREMENT_ENTER'
    $databaseName = 'lattice_task019_' + $RunId.Substring(0, 8) + '_transition'
    $encodedPassword = [Uri]::EscapeDataString($password)
    $catalogUrl = 'postgresql://task019_harness:{0}@127.0.0.1:{1}/{2}' -f `
        $encodedPassword, $Port, $databaseName
    $catalogArguments = @(
        "+$approvedRustToolchain", 'test', '-p', 'lattice-postgres-store', '--lib',
        'postgres_setup::tests::measure_v7_ingress_signatures', '--locked', '--',
        '--ignored', '--exact', '--nocapture', '--test-threads=1'
    )
    $catalogProcess = Start-Process -FilePath $cargo -ArgumentList $catalogArguments `
        -WorkingDirectory $repository -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $catalogOutputPath -RedirectStandardError $catalogErrorPath `
        -Environment @{
            LATTICE_STORE_V7_CATALOG_SIGNATURE_URL = $catalogUrl
        }
    try {
        Wait-Task094OwnedProcess -Process $catalogProcess `
            -TimeoutSeconds $CargoTimeoutSeconds `
            -TimeoutDiagnostic 'TASK094_V7_CATALOG_MEASUREMENT_TIMEOUT'
    }
    finally {
        Write-Task094RedactedFile -Path $catalogOutputPath
        Write-Task094RedactedFile -Path $catalogErrorPath
    }
    if ($catalogProcess.ExitCode -ne 0) {
        throw 'TASK094_V7_CATALOG_MEASUREMENT_FAILED'
    }
    $catalogEvidence = @(
        Get-Content -LiteralPath $catalogOutputPath -ErrorAction Stop
        Get-Content -LiteralPath $catalogErrorPath -ErrorAction Stop
    )
    $expectedCatalogLabels = @(
        'OWNED_RELATION',
        'OWNED_COLUMN',
        'OWNED_CONSTRAINT',
        'OWNED_INDEX',
        'OWNED_FUNCTION',
        'OWNED_TYPE',
        'OWNED_TABLE_ACL',
        'OWNED_FUNCTION_ACL',
        'OWNED_SCHEMA_ACL',
        'AMBIGUITY_RELATION',
        'AMBIGUITY_COLUMN',
        'AMBIGUITY_CONSTRAINT',
        'AMBIGUITY_INDEX',
        'AMBIGUITY_TABLE_ACL',
        'INGRESS_FUNCTION',
        'INGRESS_FUNCTION_ACL'
    )
    $catalogSignatures = @(
        $catalogEvidence | ForEach-Object {
            foreach ($match in [regex]::Matches(
                [string]$_,
                'STORE_V7_CATALOG_[A-Z_]+_SIGNATURE=[0-9a-f]{64}'
            )) {
                $match.Value
            }
        }
    )
    $catalogLabels = @(
        $catalogSignatures | ForEach-Object {
            if ($_ -cmatch '^STORE_V7_CATALOG_([A-Z_]+)_SIGNATURE=[0-9a-f]{64}$') {
                $Matches[1]
            }
        } | Sort-Object -Unique
    )
    if (
        $catalogSignatures.Count -ne 16 -or
        $catalogLabels.Count -ne 16 -or
        @(Compare-Object -ReferenceObject ($expectedCatalogLabels | Sort-Object) `
            -DifferenceObject $catalogLabels).Count -ne 0
    ) {
        throw 'TASK094_V7_CATALOG_MEASUREMENT_OUTPUT_REJECTED'
    }
    $catalogExactResults = @(
        $catalogEvidence |
            Where-Object { $_ -cmatch '^test result: ok\. 1 passed; 0 failed; 0 ignored;' }
    )
    if ($catalogExactResults.Count -ne 1) {
        throw 'TASK094_V7_CATALOG_MEASUREMENT_NOT_OBSERVED'
    }
    $forbiddenObjectCounts = @(
        $catalogEvidence | ForEach-Object {
            foreach ($match in [regex]::Matches(
                [string]$_,
                'STORE_V7_FORBIDDEN_SCHEMA_OBJECT_COUNTS=(?:[0-9]+,){9}[0-9]+'
            )) {
                $match.Value
            }
        }
    )
    if ($forbiddenObjectCounts.Count -ne 1) {
        throw 'TASK094_V7_FORBIDDEN_SCHEMA_OBJECT_COUNTS_REJECTED'
    }
    Write-Output 'TASK094_V7_CATALOG_MEASUREMENT_PASS signatures=16 forbidden_counts=1'
    $liveGateObserved = $true
}
catch {
    if (Test-Path -LiteralPath $logPath -PathType Leaf) {
        Write-Host 'TASK094_POSTGRES_FAILURE_LOG_ENTER'
        Get-Content -LiteralPath $logPath |
            Select-String -Pattern 'ERROR:|CONTEXT:|DETAIL:|HINT:' -Context 2,6 |
            Select-Object -Last 80 |
            ForEach-Object { ([string]$_).Replace($password, '[REDACTED]') } |
            Out-Host
        Write-Host 'TASK094_POSTGRES_FAILURE_LOG_EXIT'
    }
    throw
}
finally {
    $passwordCleanupFailed = $false
    if (Test-Path -LiteralPath $passwordPath) {
        try {
            Remove-Item -LiteralPath $passwordPath -Force
        }
        catch {
            $passwordCleanupFailed = $true
            Write-Warning 'TASK094_CREDENTIAL_FILE_CLEANUP_RETRY_REQUIRED'
        }
    }
    if (Test-Path -LiteralPath $passwordPath) {
        $passwordCleanupFailed = $true
    }
    $teardownDiagnostic = $null
    $listenerSurvivors = -1
    try {
        if (Test-Path -LiteralPath $dataRoot -PathType Container) {
            $pidPath = Join-Path $dataRoot 'postmaster.pid'
            if (Test-Path -LiteralPath $pidPath -PathType Leaf) {
                $ownedPid = [int]((Get-Content -LiteralPath $pidPath -TotalCount 1).Trim())
                $ownedProcess = Get-Process -Id $ownedPid -ErrorAction SilentlyContinue
                if ($null -ne $ownedProcess) {
                    $teardownOwner = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
                    $teardownListeners = @(Get-Task094ListenerPid)
                    $pathMatches = [IO.Path]::GetFullPath($ownedProcess.Path) -ceq `
                        [IO.Path]::GetFullPath($postgres)
                    $markerMatches = $teardownOwner.owner -ceq 'TASK-094' -and `
                        $teardownOwner.run_id -ceq $RunId -and `
                        [int]$teardownOwner.port -eq $Port
                    $listenerMatches = $teardownListeners.Count -eq 1 -and `
                        $teardownListeners[0] -eq $ownedPid
                    $savedIdentityMatches = $null -eq $ownedPostgresPid -or (
                        $ownedPid -eq $ownedPostgresPid -and
                        $ownedProcess.StartTime.ToUniversalTime().Ticks -eq $ownedPostgresStartTicks
                    )
                    if (-not ($pathMatches -and $markerMatches -and $listenerMatches -and $savedIdentityMatches)) {
                        throw 'TASK094_TEARDOWN_OWNERSHIP_LOST'
                    }
                    $stopProcess = Start-Task094Process -FilePath $pgCtl `
                        -ArgumentList @('-D', $dataRoot, '-m', 'fast', '-w', 'stop') `
                        -WorkingDirectory $runRoot
                    Wait-Task094OwnedProcess -Process $stopProcess `
                        -TimeoutSeconds $PostgresProcessTimeoutSeconds `
                        -TimeoutDiagnostic 'TASK094_PG_STOP_TIMEOUT'
                    if ($stopProcess.ExitCode -ne 0) {
                        throw 'TASK094_PG_STOP_FAILED'
                    }
                }
                $statusArguments = @('-D', $dataRoot, 'status')
                if ($InjectTeardownStatusFailure) {
                    $statusArguments += '--task094-injected-invalid-option'
                }
                $statusProcess = Start-Task094Process -FilePath $pgCtl `
                    -ArgumentList $statusArguments -WorkingDirectory $runRoot
                Wait-Task094OwnedProcess -Process $statusProcess `
                    -TimeoutSeconds $PostgresProcessTimeoutSeconds `
                    -TimeoutDiagnostic 'TASK094_PG_STATUS_TIMEOUT'
                if ($statusProcess.ExitCode -ne 3) {
                    throw 'TASK094_PG_STATUS_NOT_STOPPED'
                }
            }
        }
        $listenerSurvivors = @(Get-Task094ListenerPid).Count
        if ($listenerSurvivors -ne 0) {
            throw 'TASK094_LISTENER_SURVIVOR'
        }
    }
    catch {
        $observedTeardownDiagnostic = [string]$_.Exception.Message
        $knownTeardownDiagnostics = @(
            'TASK094_TEARDOWN_OWNERSHIP_LOST',
            'TASK094_PG_STOP_TIMEOUT',
            'TASK094_PG_STOP_FAILED',
            'TASK094_PG_STATUS_TIMEOUT',
            'TASK094_PG_STATUS_NOT_STOPPED',
            'TASK094_LISTENER_SURVIVOR',
            'TASK094_PROCESS_TREE_EXIT_OBSERVATION_FAILED',
            'TASK094_PROCESS_TREE_SURVIVOR'
        )
        $teardownDiagnostic = if ($knownTeardownDiagnostics -ccontains $observedTeardownDiagnostic) {
            $observedTeardownDiagnostic
        }
        else {
            'TASK094_TEARDOWN_OBSERVATION_FAILED'
        }
    }
    $teardownIncomplete = $null -ne $teardownDiagnostic -or `
        -not $script:task094ProcessTreeCleanupProven
    foreach ($evidencePath in @(
        $logPath,
        $cargoOutputPath,
        $cargoErrorPath,
        $catalogOutputPath,
        $catalogErrorPath
    )) {
        if (Test-Path -LiteralPath $evidencePath -PathType Leaf) {
            try {
                $evidenceText = [IO.File]::ReadAllText($evidencePath)
                if ($evidenceText.Contains($password)) {
                    $sanitizedText = $evidenceText.Replace($password, '[REDACTED]')
                    [IO.File]::WriteAllText(
                        $evidencePath,
                        $sanitizedText,
                        [Text.UTF8Encoding]::new($false)
                    )
                }
                if ([IO.File]::ReadAllText($evidencePath).Contains($password)) {
                    $passwordCleanupFailed = $true
                }
            }
            catch {
                $passwordCleanupFailed = $true
                Write-Warning 'TASK094_FAILURE_EVIDENCE_SCRUB_RETRY_REQUIRED'
            }
        }
    }
    if (Test-Path -LiteralPath $runRoot) {
        $deleteTarget = [IO.Path]::GetFullPath($runRoot)
        if ($deleteTarget -cne $expectedRoot) {
            throw 'TASK094_DELETE_TARGET_MISMATCH'
        }
        if ($teardownIncomplete) {
            Write-Output "TASK094_FAILURE_EVIDENCE_PRESERVED run_root=$deleteTarget"
        }
        elseif ($passwordCleanupFailed) {
            try {
                Remove-Item -LiteralPath $deleteTarget -Recurse -Force
            }
            catch {
                Write-Warning 'TASK094_CREDENTIAL_FILE_CLEANUP_RETRY_REQUIRED'
            }
        }
        elseif ($liveGateObserved) {
            Remove-Item -LiteralPath $deleteTarget -Recurse -Force
        }
        else {
            Write-Output "TASK094_FAILURE_EVIDENCE_PRESERVED run_root=$deleteTarget"
        }
    }
    if ($null -eq $teardownDiagnostic -and $script:task094ProcessTreeCleanupProven) {
        Write-Output "TASK094_TEARDOWN_OK root_absent=$(-not (Test-Path -LiteralPath $runRoot)) listener_survivors=$listenerSurvivors"
    }
    else {
        Write-Output "TASK094_TEARDOWN_INCOMPLETE root_absent=$(-not (Test-Path -LiteralPath $runRoot)) listener_survivors=$listenerSurvivors"
    }
    if ($null -ne $teardownDiagnostic) {
        throw $teardownDiagnostic
    }
    if (-not $teardownIncomplete -and $passwordCleanupFailed) {
        throw 'TASK094_CREDENTIAL_FILE_CLEANUP_FAILED'
    }
}

if (-not $liveGateObserved -or (Test-Path -LiteralPath $runRoot)) {
    throw 'TASK094_FINAL_GATE_NOT_CLEAN'
}
Write-Output 'TASK094_FOCUSED_LIVE_GATE_PASS'
