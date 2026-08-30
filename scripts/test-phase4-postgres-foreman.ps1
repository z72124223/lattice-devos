#requires -Version 7.0

<#
.SYNOPSIS
Runs the ordered disposable PostgreSQL acceptance for the Phase 4 foreman.

.DESCRIPTION
Creates one marker-owned PostgreSQL 17 cluster on an ephemeral loopback port.
It uses the formal product bootstrap path (Store-v5 foundation, Memory/Writer,
Store-v6/v7, and Foreman extension) before general-submission and Foreman live
tests run in order: install/ACL, physical restart replay, atomic four-worker
capacity with a deliberate Ledger-before-artifact failpoint, a second physical
restart that finalizes the exact staged artifact, and destructive catalog
tamper last.
#>

[CmdletBinding()]
param(
    [switch]$KeepArtifacts,
    [switch]$StopAfterStoreV7Fresh,
    [switch]$MeasureCatalogPins,
    [switch]$StaticPreflight,
    [ValidateRange(30, 1800)]
    [int]$StageTimeoutSeconds = 600
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$utf8 = [Text.UTF8Encoding]::new($false)
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$requiredRustToolchain = '1.97.1-x86_64-pc-windows-msvc'
$requiredCargoSha256 = 'ddfbad20b31b918d3439d070945ec59bbfe037a6ec0ab5b584459e69c8b37d1b'
$env:RUSTUP_TOOLCHAIN = $requiredRustToolchain
$postgresBin = 'C:\Program Files\PostgreSQL\17\bin'
$initdb = Join-Path $postgresBin 'initdb.exe'
$pgCtl = Join-Path $postgresBin 'pg_ctl.exe'
$postgres = Join-Path $postgresBin 'postgres.exe'
$rustup = [IO.Path]::GetFullPath((Get-Command rustup.exe -ErrorAction Stop).Source)
$cargoPath = (& $rustup which cargo --toolchain $requiredRustToolchain).Trim()
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $cargoPath -PathType Leaf)) {
    throw 'PHASE4_POSTGRES_CARGO_RESOLUTION_FAILED'
}
$cargo = [IO.Path]::GetFullPath($cargoPath)
$cargoSuffix = [IO.Path]::Combine(
    '.rustup', 'toolchains', $requiredRustToolchain, 'bin', 'cargo.exe'
)
$cargoIdentity = (& $cargo -Vv) -join "`n"
if ($LASTEXITCODE -ne 0 -or
    -not $cargo.EndsWith($cargoSuffix, [StringComparison]::OrdinalIgnoreCase) -or
    (Get-FileHash -LiteralPath $cargo -Algorithm SHA256).Hash.ToLowerInvariant() -cne
        $requiredCargoSha256 -or
    $cargoIdentity -cnotmatch '(?m)^release: 1\.97\.1$' -or
    $cargoIdentity -cnotmatch '(?m)^host: x86_64-pc-windows-msvc$') {
    throw 'PHASE4_POSTGRES_CARGO_IDENTITY_REJECTED'
}
$foremanSql = [IO.File]::ReadAllText(
    (Join-Path $repositoryRoot 'db\extensions\foreman-execution\v1.sql'),
    $utf8
)
$expectedMeasurementShape = [ordered]@{
    table = [regex]::Matches(
        $foremanSql,
        '(?m)^[^\S\r\n]*CREATE TABLE foreman_execution\.'
    ).Count
    function = [regex]::Matches(
        $foremanSql,
        '(?m)^[^\S\r\n]*CREATE FUNCTION foreman_execution\.'
    ).Count
    hardened_function = [regex]::Matches(
        $foremanSql,
        '(?m)^[^\S\r\n]*SECURITY DEFINER[^\S\r\n]*$'
    ).Count
    runtime_execute = [regex]::Matches(
        $foremanSql,
        '(?m)^[^\S\r\n]*GRANT EXECUTE ON FUNCTION foreman_execution\.'
    ).Count
}
if ($expectedMeasurementShape.table -le 0 -or
    $expectedMeasurementShape.function -le 0 -or
    $expectedMeasurementShape.hardened_function -ne $expectedMeasurementShape.function -or
    $expectedMeasurementShape.runtime_execute -le 0 -or
    $expectedMeasurementShape.runtime_execute -gt $expectedMeasurementShape.function) {
    throw 'PHASE4_POSTGRES_STATIC_CATALOG_SHAPE_REJECTED'
}
$measurementReceiptSchema = 'lattice.phase4-foreman-catalog-measurement.v1'
$postgresReceiptSchema = 'lattice.phase4-postgres-live.v1'
$receiptSchema = if ($MeasureCatalogPins) {
    $measurementReceiptSchema
}
else {
    $postgresReceiptSchema
}
$ownerKind = if ($MeasureCatalogPins) {
    'LATTICE_PHASE4_FOREMAN_CATALOG_MEASUREMENT_V1'
}
else {
    'LATTICE_PHASE4_POSTGRES_FOREMAN_ACCEPTANCE_V1'
}

if ($MeasureCatalogPins -and $StopAfterStoreV7Fresh) {
    throw 'PHASE4_POSTGRES_MEASUREMENT_MODE_CONFLICT'
}

function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Remove-InitdbPassword {
    if ([string]::IsNullOrEmpty($script:passwordPath)) { return }
    $resolvedPasswordPath = [IO.Path]::GetFullPath($script:passwordPath)
    $expectedPasswordPath = [IO.Path]::GetFullPath(
        (Join-Path $script:runRoot '.initdb-password')
    )
    if ($resolvedPasswordPath -cne $expectedPasswordPath -or
        [IO.Path]::GetFileName($resolvedPasswordPath) -cne '.initdb-password' -or
        -not $resolvedPasswordPath.StartsWith(
            $script:runRoot + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase
        )) {
        throw 'PHASE4_POSTGRES_PASSWORD_PATH_REJECTED'
    }
    [IO.File]::Delete($resolvedPasswordPath)
    if ([IO.File]::Exists($resolvedPasswordPath)) {
        throw 'PHASE4_POSTGRES_PASSWORD_CLEANUP_FAILED'
    }
}

function Test-InitdbPasswordCleanup {
    $probeRoot = Join-Path (
        [IO.Path]::GetTempPath()
    ) ('lattice-phase4-password-cleanup-probe-' + [Guid]::NewGuid().ToString('N'))
    $foreignRoot = $probeRoot + '-foreign'
    try {
        [IO.Directory]::CreateDirectory($probeRoot) | Out-Null
        [IO.Directory]::CreateDirectory($foreignRoot) | Out-Null
        $script:runRoot = [IO.Path]::GetFullPath($probeRoot)
        $script:passwordPath = Join-Path $script:runRoot '.initdb-password'
        [IO.File]::WriteAllBytes(
            $script:passwordPath,
            [Text.Encoding]::UTF8.GetBytes('deterministic-cleanup-probe')
        )
        Remove-InitdbPassword
        if ([IO.File]::Exists($script:passwordPath)) {
            throw 'PHASE4_POSTGRES_PASSWORD_CLEANUP_SELF_TEST_FAILED'
        }

        $foreignPasswordPath = Join-Path $foreignRoot '.initdb-password'
        [IO.File]::WriteAllBytes(
            $foreignPasswordPath,
            [Text.Encoding]::UTF8.GetBytes('must-not-be-deleted')
        )
        $script:passwordPath = $foreignPasswordPath
        $rejection = $null
        try { Remove-InitdbPassword }
        catch { $rejection = $_.Exception.Message }
        if ($rejection -cne 'PHASE4_POSTGRES_PASSWORD_PATH_REJECTED' -or
            -not [IO.File]::Exists($foreignPasswordPath)) {
            throw 'PHASE4_POSTGRES_PASSWORD_NEGATIVE_SELF_TEST_FAILED'
        }
        [IO.File]::Delete($foreignPasswordPath)
    }
    finally {
        foreach ($candidate in @(
            (Join-Path $probeRoot '.initdb-password'),
            (Join-Path $foreignRoot '.initdb-password')
        )) {
            if ([IO.File]::Exists($candidate)) { [IO.File]::Delete($candidate) }
        }
        foreach ($candidateRoot in @($probeRoot, $foreignRoot)) {
            if ([IO.Directory]::Exists($candidateRoot)) {
                if (@([IO.Directory]::EnumerateFileSystemEntries($candidateRoot)).Count -ne 0) {
                    throw 'PHASE4_POSTGRES_PASSWORD_SELF_TEST_CLEANUP_FAILED'
                }
                [IO.Directory]::Delete($candidateRoot, $false)
            }
        }
    }
}

if ($StaticPreflight) {
    Test-InitdbPasswordCleanup
    [ordered]@{
        schema = 'lattice.phase4-postgres-foreman-static-preflight.v1'
        status = 'PASS'
        rust_toolchain = $requiredRustToolchain
        table_count = $expectedMeasurementShape.table
        function_count = $expectedMeasurementShape.function
        hardened_function_count = $expectedMeasurementShape.hardened_function
        runtime_execute_count = $expectedMeasurementShape.runtime_execute
    } | ConvertTo-Json -Compress
    exit 0
}

function Invoke-CargoStage([string]$Name, [string[]]$Argument) {
    $stdoutPath = Join-Path $script:runRoot ('cargo-' + $Name + '.stdout.log')
    $stderrPath = Join-Path $script:runRoot ('cargo-' + $Name + '.stderr.log')
    Write-Output ('PHASE4_POSTGRES_STAGE_BEGIN:' + $Name)
    $script:currentStage = ('CARGO_' + $Name)
    $process = Start-Process -FilePath $script:cargo -ArgumentList $Argument `
        -WorkingDirectory $script:repositoryRoot -NoNewWindow -PassThru `
        -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($script:stageTimeoutSeconds)
    while (-not $process.HasExited -and [DateTimeOffset]::UtcNow -lt $deadline) {
        $process.Refresh()
        $null = $process.WaitForExit(1000)
    }
    if (-not $process.HasExited) {
        try { $process.Kill($true) } catch { }
        throw ('PHASE4_POSTGRES_STAGE_TIMEOUT_' + $Name)
    }
    if ($process.ExitCode -ne 0) {
        throw ('PHASE4_POSTGRES_STAGE_FAILED_' + $Name)
    }
    $script:stages.Add($Name)
    Write-Output ('PHASE4_POSTGRES_STAGE_PASS:' + $Name)
}

function Start-OwnedPostgres {
    Write-Host 'PHASE4_POSTGRES_START_BEGIN'
    $launchArguments = '-D "{0}" -l "{1}" -o "{2}" -W start' -f `
        $script:dataRoot, $script:logPath, $script:serverOptions
    $launcher = Start-Process -FilePath $script:pgCtl -ArgumentList $launchArguments `
        -NoNewWindow -PassThru
    if (-not $launcher.WaitForExit(30000)) {
        try { $launcher.Kill($true) } catch { }
        throw 'PHASE4_POSTGRES_START_LAUNCH_TIMEOUT'
    }
    if ($launcher.ExitCode -ne 0) { throw 'PHASE4_POSTGRES_START_FAILED' }
    $ready = $false
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(30)
    while (-not $ready -and [DateTimeOffset]::UtcNow -lt $deadline) {
        $probe = [Net.Sockets.TcpClient]::new()
        try {
            $connected = $probe.ConnectAsync('127.0.0.1', $script:port)
            $ready = $connected.Wait(1000) -and $probe.Connected
        }
        finally {
            $probe.Dispose()
        }
    }
    if (-not $ready) { throw 'PHASE4_POSTGRES_START_TIMEOUT' }
    $script:postgresRunning = $true
    $pidPath = Join-Path $script:dataRoot 'postmaster.pid'
    $pidText = (Get-Content -LiteralPath $pidPath -TotalCount 1).Trim()
    if ($pidText -cnotmatch '\A[1-9][0-9]*\z') { throw 'PHASE4_POSTGRES_PID_REJECTED' }
    $process = Get-Process -Id ([int]$pidText) -ErrorAction Stop
    if ([IO.Path]::GetFullPath($process.Path) -cne [IO.Path]::GetFullPath($script:postgres)) {
        throw 'PHASE4_POSTGRES_PROCESS_REJECTED'
    }
    $started = [pscustomobject]@{
        pid = [int]$pidText
        started_utc_ticks = [long]$process.StartTime.ToUniversalTime().Ticks
    }
    Write-Host ('PHASE4_POSTGRES_START_PASS:PID=' + $started.pid)
    return $started
}

function Stop-OwnedPostgres {
    if (-not $script:postgresRunning) { return }
    $pidPath = Join-Path $script:dataRoot 'postmaster.pid'
    if (-not (Test-Path -LiteralPath $pidPath -PathType Leaf)) {
        throw 'PHASE4_POSTGRES_STOP_OWNERSHIP_REJECTED'
    }
    & $script:pgCtl -D $script:dataRoot -m fast -w stop
    if ($LASTEXITCODE -ne 0) { throw 'PHASE4_POSTGRES_STOP_FAILED' }
    $script:postgresRunning = $false
}

foreach ($binary in @($initdb, $pgCtl, $postgres, $cargo)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw 'PHASE4_POSTGRES_REQUIRED_BINARY_MISSING'
    }
}

$startedAt = [DateTimeOffset]::UtcNow
$runId = ([Guid]::NewGuid().ToString('N')).ToLowerInvariant()
$tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
    [IO.Path]::DirectorySeparatorChar
)
$rootName = if ($MeasureCatalogPins) {
    'lattice-phase4-catalog-measure-' + $runId
}
else {
    'lattice-phase4-pg-live-' + $runId
}
$runRoot = [IO.Path]::GetFullPath((Join-Path $tempParent $rootName))
$expectedRoot = [IO.Path]::GetFullPath((Join-Path $tempParent $rootName))
$script:dataRoot = Join-Path $runRoot 'data'
$script:runRoot = $runRoot
$script:repositoryRoot = $repositoryRoot
$script:stageTimeoutSeconds = $StageTimeoutSeconds
$script:logPath = Join-Path $runRoot 'postgres.log'
$markerPath = Join-Path $runRoot $(if ($MeasureCatalogPins) {
    '.phase4-catalog-measure-owner.json'
} else {
    '.phase4-postgres-owner.json'
})
$passwordPath = Join-Path $runRoot '.initdb-password'
$script:passwordPath = $passwordPath
$password = (([Guid]::NewGuid().ToString('N')) + ([Guid]::NewGuid().ToString('N'))).ToLowerInvariant()
$databaseName = if (-not $MeasureCatalogPins) {
    'lattice_task019_' + $runId.Substring(0, 8) + '_base'
}
else { $null }
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = [int]([Net.IPEndPoint]$listener.LocalEndpoint).Port
$listener.Stop()
$script:port = $port
$script:serverOptions = "-p $port -h 127.0.0.1 -c ssl=off -c fsync=on " +
    '-c synchronous_commit=on -c full_page_writes=on -c max_prepared_transactions=0'
$script:postgresRunning = $false
$script:stages = [Collections.Generic.List[string]]::new()
$script:currentStage = 'PREPARE'
$cleanup = $false
$result = $null
$exitCode = 0

try {
    if ($runRoot -cne $expectedRoot -or (Test-Path -LiteralPath $runRoot) -or
        -not $runRoot.StartsWith(
            $tempParent + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase
        )) {
        throw 'PHASE4_POSTGRES_ROOT_REJECTED'
    }
    [IO.Directory]::CreateDirectory($script:dataRoot) | Out-Null
    $marker = [ordered]@{
        owner = $ownerKind
        run_id = $runId
        root = $runRoot
        port = $port
        postgres = [IO.Path]::GetFullPath($postgres)
        postgres_sha256 = Get-Sha256 $postgres
    }
    if (-not $MeasureCatalogPins) {
        $marker['database'] = $databaseName
    }
    [IO.File]::WriteAllText($markerPath, ($marker | ConvertTo-Json -Compress), $utf8)
    [IO.File]::WriteAllText($passwordPath, $password, [Text.Encoding]::ASCII)
    & $initdb -D $script:dataRoot -U runtime_bootstrap --auth-host=scram-sha-256 `
        --auth-local=trust --encoding=UTF8 --locale=C --data-checksums `
        ('--pwfile=' + $passwordPath)
    if ($LASTEXITCODE -ne 0) { throw 'PHASE4_POSTGRES_INITDB_FAILED' }
    Remove-InitdbPassword

    $first = Start-OwnedPostgres
    Write-Output 'PHASE4_POSTGRES_ENVIRONMENT_CONFIGURING'
    $script:currentStage = 'PRODUCT_ENVIRONMENT'
    $env:LATTICE_TASK019_HOST = '127.0.0.1'
    $env:LATTICE_TASK019_PORT = [string]$port
    $env:LATTICE_TASK019_RUN_ID = $runId
    $env:LATTICE_TASK019_PASSWORD = $password
    $env:LATTICE_FULL_CHAIN_RUN_MODE = 'RESUME_EXISTING'
    $env:LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
    $env:LATTICE_DELIVERY_TIMEOUT_SECONDS = '120'
    Write-Output 'PHASE4_POSTGRES_PRODUCT_ENVIRONMENT_DELIVERY_CONFIGURED'
    # Match the independently verified submission fixture authority so that
    # product bootstrap restores it without a test-side admission rewrite.
    $env:LATTICE_STORE_DAEMON_INSTANCE_ID = 'task050-fresh-process'
    $env:LATTICE_STORE_DAEMON_EPOCH = '50'
    $env:LATTICE_STORE_AUTHORITY_REVISION = '50'
    Write-Output 'PHASE4_POSTGRES_PRODUCT_ENVIRONMENT_AUTHORITY_CONFIGURED'
    $observationDigest = ('a' * 64)
    Write-Output 'PHASE4_POSTGRES_PRODUCT_ENVIRONMENT_OBSERVATION_DIGESTED'
    $headDigest = ('b' * 64)
    Write-Output 'PHASE4_POSTGRES_PRODUCT_ENVIRONMENT_HEAD_DIGESTED'
    $env:LATTICE_STORE_OBSERVATION_DIGEST = $observationDigest
    $env:LATTICE_STORE_AUTHORITY_HEAD_DIGEST = $headDigest
    $env:LATTICE_MANAGED_FOREMAN_MODE = 'DISABLED'
    Write-Output 'PHASE4_POSTGRES_PRODUCT_ENVIRONMENT_CONFIGURED'
    Invoke-CargoStage 'PRODUCT_POSTGRES_INITIALIZE' @(
        'run', '-p', 'lattice-runtime', '--bin', 'latticed', '--locked', '--offline',
        '--', '--postgres-initialize'
    )
    if ($MeasureCatalogPins) {
        $env:LATTICE_PHASE4_FOREMAN_CATALOG_MEASUREMENT_LIVE = '1'
        $env:LATTICE_PHASE4_FOREMAN_CATALOG_MEASUREMENT_ROOT = $runRoot
        Invoke-CargoStage 'FOREMAN_CATALOG_MEASUREMENT' @(
            'test', '-p', 'lattice-runtime', '--lib', '--locked', '--offline',
            'composition::tests::disposable_store_v7_foreman_catalog_measurement_profile',
            '--', '--ignored', '--exact', '--nocapture'
        )
        $measurementLog = Join-Path $runRoot 'cargo-FOREMAN_CATALOG_MEASUREMENT.stdout.log'
        $measurementText = [IO.File]::ReadAllText($measurementLog, $utf8)
        $databaseMatch = [regex]::Match(
            $measurementText,
            '(?m)^FOREMAN_CATALOG_DATABASE=([a-z0-9_]{3,63})$'
        )
        if (-not $databaseMatch.Success) {
            throw 'PHASE4_POSTGRES_CATALOG_MEASUREMENT_DATABASE_REJECTED'
        }
        $measuredDatabase = $databaseMatch.Groups[1].Value
        $measurementMarker = [IO.File]::ReadAllText($markerPath, $utf8) | ConvertFrom-Json
        if ($measurementMarker.database -cne $measuredDatabase) {
            throw 'PHASE4_POSTGRES_CATALOG_MEASUREMENT_DATABASE_REJECTED'
        }
        $env:LATTICE_FOREMAN_CATALOG_SIGNATURE_URL = (
            'postgresql://lattice_migrator_login:{0}@127.0.0.1:{1}/{2}?sslmode=disable' -f
            $password, $port, $measuredDatabase
        )
        Invoke-CargoStage 'FOREMAN_CATALOG_RUST_CALCULATOR' @(
            'test', '-p', 'lattice-postgres-foreman', '--lib', '--locked', '--offline',
            'setup::tests::measure_catalog_digests',
            '--', '--ignored', '--exact', '--nocapture'
        )
        $calculatorLog = Join-Path $runRoot 'cargo-FOREMAN_CATALOG_RUST_CALCULATOR.stdout.log'
        $calculatorText = [IO.File]::ReadAllText($calculatorLog, $utf8)
        $tableMatch = [regex]::Match($measurementText, '(?m)^FOREMAN_CATALOG_TABLE_COUNT=([0-9]+)$')
        $functionMatch = [regex]::Match($measurementText, '(?m)^FOREMAN_CATALOG_FUNCTION_COUNT=([0-9]+)$')
        $hardenedMatch = [regex]::Match(
            $measurementText,
            '(?m)^FOREMAN_CATALOG_HARDENED_FUNCTION_COUNT=([0-9]+)$'
        )
        $runtimeMatch = [regex]::Match(
            $measurementText,
            '(?m)^FOREMAN_CATALOG_RUNTIME_EXECUTE_COUNT=([0-9]+)$'
        )
        $functionDigestMatch = [regex]::Match(
            $calculatorText,
            '(?m)^FOREMAN_FUNCTION_CATALOG_SHA256=([0-9a-f]{64})$'
        )
        $tableDigestMatch = [regex]::Match(
            $calculatorText,
            '(?m)^FOREMAN_TABLE_CATALOG_SHA256=([0-9a-f]{64})$'
        )
        if (-not $databaseMatch.Success -or -not $tableMatch.Success -or
            -not $functionMatch.Success -or
            -not $hardenedMatch.Success -or -not $runtimeMatch.Success -or
            -not $functionDigestMatch.Success -or -not $tableDigestMatch.Success) {
            throw 'PHASE4_POSTGRES_CATALOG_MEASUREMENT_OUTPUT_REJECTED'
        }
        $tableCount = [int64]$tableMatch.Groups[1].Value
        $functionCount = [int64]$functionMatch.Groups[1].Value
        $hardenedFunctionCount = [int64]$hardenedMatch.Groups[1].Value
        $runtimeExecuteCount = [int64]$runtimeMatch.Groups[1].Value
        if ($tableCount -ne $expectedMeasurementShape.table -or
            $functionCount -ne $expectedMeasurementShape.function -or
            $hardenedFunctionCount -ne $expectedMeasurementShape.hardened_function -or
            $runtimeExecuteCount -ne $expectedMeasurementShape.runtime_execute) {
            throw 'PHASE4_POSTGRES_CATALOG_MEASUREMENT_SHAPE_REJECTED'
        }
        $result = [ordered]@{
            schema = $receiptSchema
            status = 'PASS'
            run_id = $runId
            database = $measuredDatabase
            port = $port
            evidence_root = $runRoot
            table_count = $tableCount
            function_count = $functionCount
            hardened_function_count = $hardenedFunctionCount
            runtime_execute_count = $runtimeExecuteCount
            function_catalog_sha256 = $functionDigestMatch.Groups[1].Value
            table_catalog_sha256 = $tableDigestMatch.Groups[1].Value
            stages = @($script:stages)
        }
        [IO.File]::WriteAllText(
            (Join-Path $runRoot 'catalog-pin-result.json'),
            ($result | ConvertTo-Json -Depth 10),
            $utf8
        )
    }
    else {
    Invoke-CargoStage 'PRODUCT_POSTGRES_BOOTSTRAP' @(
        'run', '-p', 'lattice-runtime', '--bin', 'latticed', '--locked', '--offline',
        '--', '--postgres-bootstrap'
    )
    $env:LATTICE_TASK_SUBMISSION_LIVE = '1'
    $env:LATTICE_TASK_SUBMISSION_PROVISION_FRESH = '0'
    $env:LATTICE_TASK_SUBMISSION_SUPERUSER = 'runtime_bootstrap'
    $env:LATTICE_TASK_SUBMISSION_DATABASE = $databaseName
    $env:LATTICE_TASK_SUBMISSION_RUN_ID = $runId
    Invoke-CargoStage 'STORE_V7_FRESH' @(
        'test', '-p', 'lattice-postgres-store', '--test', 'postgres_task_ledger',
        '--locked', '--offline',
        'general_submission_is_atomic_idempotent_and_fresh_reconnectable_when_provisioned',
        '--', '--exact', '--nocapture'
    )

    if (-not $StopAfterStoreV7Fresh) {
    $env:LATTICE_FOREMAN_LIVE = '1'
    $env:LATTICE_FOREMAN_DATABASE_NAME = $databaseName
    $env:LATTICE_FOREMAN_RUN_ID = $runId
    $env:LATTICE_FOREMAN_MIGRATOR_URL = (
        'postgresql://lattice_migrator_login:{0}@127.0.0.1:{1}/{2}?sslmode=disable' -f
        $password, $port, $databaseName
    )
    $env:LATTICE_FOREMAN_RUNTIME_URL = (
        'postgresql://lattice_runtime_login:{0}@127.0.0.1:{1}/{2}?sslmode=disable' -f
        $password, $port, $databaseName
    )
    Invoke-CargoStage 'FOREMAN_APPLY_ACL_RECONNECT' @(
        'test', '-p', 'lattice-postgres-foreman', '--test', 'postgres_live',
        '--locked', '--offline', 'disposable_store_v7_bootstrap_owned_extension_apply_acl_and_reconnect',
        '--', '--ignored', '--exact', '--nocapture'
    )
    Invoke-CargoStage 'FOREMAN_ARTIFACT_INGRESS_GUARDS' @(
        'test', '-p', 'lattice-postgres-foreman', '--test', 'postgres_live',
        '--locked', '--offline',
        'disposable_store_v7_artifact_ingress_guards_reject_before_insert',
        '--', '--ignored', '--exact', '--nocapture'
    )

    Stop-OwnedPostgres
    $second = Start-OwnedPostgres
    if ($first.pid -eq $second.pid -and
        $first.started_utc_ticks -eq $second.started_utc_ticks) {
        throw 'PHASE4_POSTGRES_PROCESS_NOT_RESTARTED'
    }
    Invoke-CargoStage 'FOREMAN_FRESH_PROCESS_REPLAY' @(
        'test', '-p', 'lattice-postgres-foreman', '--test', 'postgres_live',
        '--locked', '--offline', 'disposable_store_v7_fresh_process_restart_replay',
        '--', '--ignored', '--exact', '--nocapture'
    )
    Invoke-CargoStage 'FOREMAN_ATOMIC_CAPACITY' @(
        'test', '-p', 'lattice-postgres-foreman', '--test', 'postgres_live',
        '--release', '--locked', '--offline',
        'disposable_store_v7_atomic_claim_capacity_and_retry_budget',
        '--', '--ignored', '--exact', '--nocapture'
    )
    Stop-OwnedPostgres
    $third = Start-OwnedPostgres
    if ($second.pid -eq $third.pid -and
        $second.started_utc_ticks -eq $third.started_utc_ticks) {
        throw 'PHASE4_POSTGRES_ARTIFACT_PROCESS_NOT_RESTARTED'
    }
    Invoke-CargoStage 'FOREMAN_APPROVAL_OWNER_RESTART_TAMPER_ACL' @(
        'test', '-p', 'lattice-postgres-foreman', '--test', 'postgres_live',
        '--locked', '--offline',
        'disposable_store_v7_approval_owner_snapshot_restart_tamper_and_acl',
        '--', '--ignored', '--exact', '--nocapture'
    )
    Invoke-CargoStage 'FOREMAN_ARTIFACT_OUTBOX_RESTART' @(
        'test', '-p', 'lattice-postgres-foreman', '--test', 'postgres_live',
        '--locked', '--offline',
        'disposable_store_v7_fresh_process_staged_artifact_finalize_replay',
        '--', '--ignored', '--exact', '--nocapture'
    )
    Invoke-CargoStage 'FOREMAN_CATALOG_TAMPER' @(
        'test', '-p', 'lattice-postgres-foreman', '--test', 'postgres_live',
        '--locked', '--offline', 'disposable_store_v7_catalog_tamper_fails_closed',
        '--', '--ignored', '--exact', '--nocapture'
    )
    $result = [ordered]@{
        schema = $receiptSchema
        status = 'PASS'
        run_id = $runId
        database = $databaseName
        port = $port
        postgres_restart = $true
        first_pid = $first.pid
        second_pid = $second.pid
        third_pid = $third.pid
        stages = @($script:stages)
    }
    }
    if ($StopAfterStoreV7Fresh) {
        $result = [ordered]@{
            schema = $receiptSchema
            status = 'PASS'
            run_id = $runId
            database = $databaseName
            port = $port
            postgres_restart = $false
            stages = @($script:stages)
        }
    }
    }
}
catch {
    $exitCode = 1
    $failure = if ($_.Exception.Message -cmatch '\A[A-Z][A-Z0-9_]{0,127}\z') {
        $_.Exception.Message
    }
    else {
        'PHASE4_POSTGRES_STAGE_FAILED_' + $script:currentStage
    }
    $result = [ordered]@{
        schema = $receiptSchema
        status = 'FAIL'
        failure = $failure
        completed_stages = @($script:stages)
        evidence_root = $runRoot
    }
}
finally {
    try { Remove-InitdbPassword }
    catch {
        $exitCode = 1
        $result['status'] = 'FAIL'
        $result['failure'] = 'PHASE4_POSTGRES_PASSWORD_CLEANUP_FAILED'
    }
    try { Stop-OwnedPostgres }
    catch {
        $exitCode = 1
        if ($result.status -ceq 'PASS') {
            $result['status'] = 'FAIL'
            $result['failure'] = 'PHASE4_POSTGRES_FINAL_STOP_FAILED'
        }
    }
    # A failed live gate is durable evidence. Keep its exact owned root unless
    # the caller explicitly reruns and obtains PASS; successful disposable
    # profiles remain self-cleaning by default.
    if (-not $KeepArtifacts -and -not $MeasureCatalogPins -and
        $result.status -ceq 'PASS' -and -not $script:postgresRunning) {
        try {
            $stored = [IO.File]::ReadAllText($markerPath, $utf8) | ConvertFrom-Json
            $resolved = [IO.Path]::GetFullPath($runRoot)
            if ($resolved -cne $expectedRoot -or $stored.owner -cne $ownerKind -or
                $stored.run_id -cne $runId -or $stored.root -cne $resolved -or
                [IO.Path]::GetFileName($resolved) -cne $rootName -or
                -not $resolved.StartsWith(
                    $tempParent + [IO.Path]::DirectorySeparatorChar,
                    [StringComparison]::OrdinalIgnoreCase
                )) {
                throw 'PHASE4_POSTGRES_DELETE_TARGET_REJECTED'
            }
            Remove-Item -LiteralPath $resolved -Recurse -Force
            $cleanup = -not (Test-Path -LiteralPath $resolved)
        }
        catch {
            $exitCode = 1
            if ($result.status -ceq 'PASS') {
                $result['status'] = 'FAIL'
                $result['failure'] = 'PHASE4_POSTGRES_CLEANUP_FAILED'
            }
        }
    }
}

$result['elapsed_ms'] = [long](([DateTimeOffset]::UtcNow - $startedAt).TotalMilliseconds)
$result['cleanup'] = $cleanup
$result | ConvertTo-Json -Compress -Depth 10
exit $exitCode
