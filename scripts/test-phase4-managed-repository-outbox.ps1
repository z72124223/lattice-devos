#requires -Version 7.0

<#
.SYNOPSIS
Runs only the concrete managed-repository Artifact outbox crash-window tests.

.DESCRIPTION
Creates one marker-owned PostgreSQL 17 cluster on an ephemeral loopback port,
bootstraps the existing Store-v7 product profile, and runs the ignored
`PostgresManagedForemanRepository` recovery and exact-replay tests serially.
It does not run the broader Phase-4 eight-stage PostgreSQL acceptance.
#>

[CmdletBinding()]
param(
    [ValidateRange(30, 1800)]
    [int]$StageTimeoutSeconds = 600
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$utf8 = [Text.UTF8Encoding]::new($false)
$script:windowsOemCodePage = (Get-Culture).TextInfo.OEMCodePage
if ($script:windowsOemCodePage -lt 1 -or $script:windowsOemCodePage -gt 65535) {
    throw 'PHASE4_MANAGED_REPOSITORY_WINDOWS_OEM_CODE_PAGE_REJECTED'
}
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$requiredRustToolchain = '1.97.1-x86_64-pc-windows-msvc'
$env:RUSTUP_TOOLCHAIN = $requiredRustToolchain
$postgresBin = 'C:\Program Files\PostgreSQL\17\bin'
$initdb = Join-Path $postgresBin 'initdb.exe'
$pgCtl = Join-Path $postgresBin 'pg_ctl.exe'
$postgres = Join-Path $postgresBin 'postgres.exe'
$psql = Join-Path $postgresBin 'psql.exe'
$rustup = [IO.Path]::GetFullPath((Get-Command rustup.exe -ErrorAction Stop).Source)
$cargo = [IO.Path]::GetFullPath(
    (& $rustup which cargo --toolchain $requiredRustToolchain).Trim()
)
$node = [IO.Path]::GetFullPath((Get-Command node.exe -ErrorAction Stop).Source)
$git = [IO.Path]::GetFullPath((Get-Command git.exe -ErrorAction Stop).Source)
$ownerKind = 'LATTICE_PHASE4_MANAGED_REPOSITORY_OUTBOX_V1'
$expectedOutboxTests = @(
    'postgres_repository_recovers_stage_before_ledger_without_provider_effect',
    'postgres_repository_recovers_ledger_before_finalize_without_provider_effect',
    'postgres_repository_closes_pending_stage_before_ledger_without_provider_effect',
    'postgres_repository_closes_pending_ledger_before_finalize_without_provider_effect',
    'postgres_repository_replays_no_provider_closure_into_attempt_two_and_rejects_substitution',
    'postgres_repository_wsl_claim_exact_replays_across_fresh_process_without_provider_effect'
)
$ownedProcessHelper = Join-Path $PSScriptRoot 'phase4-owned-process.ps1'
if (-not (Test-Path -LiteralPath $ownedProcessHelper -PathType Leaf)) {
    throw 'PHASE4_MANAGED_REPOSITORY_OWNED_PROCESS_HELPER_MISSING'
}
. $ownedProcessHelper

function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-OwnedProcessIdentityAbsent(
    [int]$ProcessId,
    [long]$ProcessStartUtcTicks,
    [string]$Failure
) {
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(10)
    do {
        $current = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
        if ($null -eq $current) { return }
        try { $currentTicks = [long]$current.StartTime.ToUniversalTime().Ticks }
        catch { $currentTicks = -1 }
        if ($currentTicks -ne $ProcessStartUtcTicks) { return }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw $Failure
}

function Get-OutboxChildEnvironment([Collections.IDictionary]$Additional) {
    $environment = [ordered]@{}
    foreach ($name in @(
        'SystemRoot', 'WINDIR', 'TEMP', 'TMP', 'PATH', 'ComSpec', 'PATHEXT',
        'PROCESSOR_ARCHITECTURE', 'NUMBER_OF_PROCESSORS', 'USERPROFILE',
        'LOCALAPPDATA', 'APPDATA', 'RUSTUP_HOME', 'CARGO_HOME', 'RUSTUP_TOOLCHAIN',
        'LATTICE_TASK019_HOST', 'LATTICE_TASK019_PORT', 'LATTICE_TASK019_RUN_ID',
        'LATTICE_TASK019_PASSWORD', 'LATTICE_FULL_CHAIN_RUN_MODE',
        'LATTICE_DELIVERY_CODEX_MODE', 'LATTICE_DELIVERY_TIMEOUT_SECONDS',
        'LATTICE_STORE_DAEMON_INSTANCE_ID', 'LATTICE_STORE_DAEMON_EPOCH',
        'LATTICE_STORE_AUTHORITY_REVISION', 'LATTICE_STORE_OBSERVATION_DIGEST',
        'LATTICE_STORE_AUTHORITY_HEAD_DIGEST', 'LATTICE_MANAGED_FOREMAN_MODE',
        'LATTICE_TASK_SUBMISSION_LIVE', 'LATTICE_TASK_SUBMISSION_PROVISION_FRESH',
        'LATTICE_TASK_SUBMISSION_SUPERUSER', 'LATTICE_TASK_SUBMISSION_DATABASE',
        'LATTICE_TASK_SUBMISSION_RUN_ID', 'LATTICE_MANAGED_REPOSITORY_LIVE',
        'LATTICE_MANAGED_REPOSITORY_PORT', 'LATTICE_MANAGED_REPOSITORY_RUN_ID',
        'LATTICE_MANAGED_REPOSITORY_PASSWORD', 'LATTICE_MANAGED_REPOSITORY_ROOT',
        'LATTICE_MANAGED_REPOSITORY_NODE', 'LATTICE_MANAGED_REPOSITORY_GIT'
    )) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if ($null -ne $value) { $environment[$name] = $value }
    }
    foreach ($entry in $Additional.GetEnumerator()) {
        $environment[[string]$entry.Key] = [string]$entry.Value
    }
    $environment['NO_COLOR'] = '1'
    return $environment
}

function Invoke-OutboxOwnedProcess(
    [string]$Executable,
    [string[]]$Argument,
    [Collections.IDictionary]$Environment,
    [int]$TimeoutSeconds,
    [string]$Failure,
    [ValidateRange(1, 65535)][int]$OutputEncodingCodePage = 65001
) {
    $owned = $null
    try {
        $owned = Start-Phase4OwnedProcessJob -Executable $Executable -Argument $Argument `
            -Environment $Environment -WorkingDirectory $script:repositoryRoot -Failure $Failure `
            -OutputEncodingCodePage $OutputEncodingCodePage
        $stdoutTask = $owned.ReadStandardOutputToEndBounded(16777216)
        $stderrTask = $owned.ReadStandardErrorToEndBounded(16777216)
        $owned.StandardInput.Close()
        if (-not $owned.WaitForExit($TimeoutSeconds * 1000)) {
            Stop-Phase4OwnedProcessJob -OwnedProcess $owned -Failure ($Failure + '_TERMINATION_FAILED')
            throw $Failure
        }
        Close-Phase4OwnedProcessJob -OwnedProcess $owned -Failure ($Failure + '_TERMINATION_FAILED')
        $stdout = [string]$stdoutTask.GetAwaiter().GetResult()
        $stderr = [string]$stderrTask.GetAwaiter().GetResult()
        if ($utf8.GetByteCount($stdout) -gt 16777216 -or
            $utf8.GetByteCount($stderr) -gt 16777216) {
            throw ($Failure + '_OUTPUT_REJECTED')
        }
        return [pscustomobject][ordered]@{
            exit_code = [int]$owned.ExitCode
            stdout = $stdout
            stderr = $stderr
        }
    }
    finally {
        if ($null -ne $owned) { $owned.Dispose() }
    }
}

function Test-LoopbackPortOpen([int]$Port) {
    $probe = [Net.Sockets.TcpClient]::new()
    try {
        $connected = $probe.ConnectAsync('127.0.0.1', $Port)
        return $connected.Wait(500) -and $probe.Connected
    }
    catch { return $false }
    finally { $probe.Dispose() }
}

function Get-OwnedPostgresProcessRecord {
    $pidPath = Join-Path $script:dataRoot 'postmaster.pid'
    if (-not (Test-Path -LiteralPath $pidPath -PathType Leaf)) {
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
    }
    $lines = @(Get-Content -LiteralPath $pidPath -TotalCount 4)
    if ($lines.Count -ne 4 -or [string]$lines[0] -cnotmatch '\A[1-9][0-9]*\z' -or
        [string]$lines[2] -cnotmatch '\A[1-9][0-9]*\z' -or
        [string]$lines[3] -cnotmatch '\A[1-9][0-9]*\z' -or
        [IO.Path]::GetFullPath([string]$lines[1]) -cne
            [IO.Path]::GetFullPath($script:dataRoot) -or [int]$lines[3] -ne $script:port) {
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
    }
    $processId = [int]$lines[0]
    $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        return [pscustomobject][ordered]@{
            process_id = $processId
            process_start_utc_ticks = $null
            process = $null
        }
    }
    if ([IO.Path]::GetFullPath($process.Path) -cne [IO.Path]::GetFullPath($script:postgres)) {
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
    }
    if ($null -eq $script:postgresOwnedProcessJob -or
        -not $script:postgresOwnedProcessJob.ContainsProcessHandle($process.Handle)) {
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
    }
    $processStart = [DateTimeOffset]::new($process.StartTime.ToUniversalTime())
    if ([Math]::Abs($processStart.ToUnixTimeSeconds() - [long]$lines[2]) -gt 5) {
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
    }
    return [pscustomobject][ordered]@{
        process_id = $processId
        process_start_utc_ticks = [long]$process.StartTime.ToUniversalTime().Ticks
        process = $process
    }
}

function Invoke-CargoStage([string]$Name, [string[]]$Argument) {
    $stdoutPath = Join-Path $script:runRoot ('cargo-' + $Name + '.stdout.log')
    $stderrPath = Join-Path $script:runRoot ('cargo-' + $Name + '.stderr.log')
    Write-Output ('PHASE4_MANAGED_REPOSITORY_STAGE_BEGIN:' + $Name)
    $script:currentStage = $Name
    $process = Invoke-OutboxOwnedProcess -Executable $script:cargo -Argument $Argument `
        -Environment (Get-OutboxChildEnvironment -Additional ([ordered]@{})) `
        -TimeoutSeconds $script:stageTimeoutSeconds `
        -Failure ('PHASE4_MANAGED_REPOSITORY_STAGE_TIMEOUT_' + $Name)
    [IO.File]::WriteAllText($stdoutPath, [string]$process.stdout, $utf8)
    [IO.File]::WriteAllText($stderrPath, [string]$process.stderr, $utf8)
    foreach ($path in @($stdoutPath, $stderrPath)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw ('PHASE4_MANAGED_REPOSITORY_STAGE_EVIDENCE_MISSING_' + $Name)
        }
    }
    $stageEvidence = [ordered]@{
        stage = $Name
        stdout_path = $stdoutPath
        stdout_bytes = [long](Get-Item -LiteralPath $stdoutPath -Force).Length
        stdout_sha256 = Get-Sha256 $stdoutPath
        stderr_path = $stderrPath
        stderr_bytes = [long](Get-Item -LiteralPath $stderrPath -Force).Length
        stderr_sha256 = Get-Sha256 $stderrPath
        exit_code = [int]$process.exit_code
    }
    $script:stageEvidence.Add([pscustomobject]$stageEvidence)
    if ($process.exit_code -ne 0) {
        throw ('PHASE4_MANAGED_REPOSITORY_STAGE_FAILED_' + $Name)
    }
    $script:stages.Add($Name)
    Write-Output ('PHASE4_MANAGED_REPOSITORY_STAGE_PASS:' + $Name)
}

function Get-ManagedRepositoryProviderDispatchCount([string]$Phase) {
    if ($Phase -cnotmatch '\A[A-Z][A-Z0-9_]{1,63}\z') {
        throw 'PHASE4_MANAGED_REPOSITORY_PROVIDER_EFFECT_QUERY_REJECTED'
    }
    $output = Invoke-OutboxOwnedProcess -Executable $script:psql -Argument @(
        '-X', '-qAt', '-v', 'ON_ERROR_STOP=1', '-h', '127.0.0.1',
        '-p', [string]$script:port, '-U', 'runtime_bootstrap', '-d', $script:databaseName,
        '-c', 'SELECT pg_catalog.count(*)::text FROM ONLY foreman_execution.provider_dispatch_claims;'
    ) -Environment (Get-OutboxChildEnvironment -Additional ([ordered]@{
        PGPASSWORD = $script:password
    })) -TimeoutSeconds 30 -Failure 'PHASE4_MANAGED_REPOSITORY_PROVIDER_EFFECT_QUERY_FAILED' `
        -OutputEncodingCodePage $script:windowsOemCodePage
    $lines = @([string]$output.stdout -split '\r?\n' |
        Where-Object { $_.Length -gt 0 })
    if ([int]$output.exit_code -ne 0 -or [string]$output.stderr -ne '' -or
        $lines.Count -ne 1 -or $lines[0] -cnotmatch '\A[0-9]+\z') {
        throw 'PHASE4_MANAGED_REPOSITORY_PROVIDER_EFFECT_QUERY_FAILED'
    }
    $count = [long]$lines[0]
    $snapshot = [ordered]@{
        phase = $Phase
        provider_dispatch_count = $count
        query_sha256 = (
            [Convert]::ToHexString(
                [Security.Cryptography.SHA256]::HashData(
                    $utf8.GetBytes($Phase + ':' + [string]$count)
                )
            ).ToLowerInvariant()
        )
    }
    $script:providerDispatchSnapshots.Add([pscustomobject]$snapshot)
    return $count
}

function Assert-ManagedRepositoryOutboxCargoEvidence($Evidence) {
    if ($null -eq $Evidence -or [string]$Evidence.stage -cne
            'MANAGED_REPOSITORY_OUTBOX_CRASH_WINDOWS' -or
        [long]$Evidence.stdout_bytes -gt 16777216 -or
        [long]$Evidence.stderr_bytes -gt 16777216) {
        throw 'PHASE4_MANAGED_REPOSITORY_TEST_EVIDENCE_REJECTED'
    }
    $stdout = [IO.File]::ReadAllText([string]$Evidence.stdout_path, $utf8)
    $stderr = [IO.File]::ReadAllText([string]$Evidence.stderr_path, $utf8)
    if ((Get-Sha256 ([string]$Evidence.stdout_path)) -cne [string]$Evidence.stdout_sha256 -or
        (Get-Sha256 ([string]$Evidence.stderr_path)) -cne [string]$Evidence.stderr_sha256) {
        throw 'PHASE4_MANAGED_REPOSITORY_TEST_EVIDENCE_REJECTED'
    }
    $combined = ($stdout + "`n" + $stderr).Replace("`r", '')
    $matches = [regex]::Matches(
        $combined,
        '(?m)^test [A-Za-z0-9_:]+::(?<name>postgres_repository_[A-Za-z0-9_]+) \.\.\. ok$'
    )
    $observed = @($matches | ForEach-Object { [string]$_.Groups['name'].Value } |
        Sort-Object -CaseSensitive -Unique)
    $expected = @($expectedOutboxTests | Sort-Object -CaseSensitive)
    if ($matches.Count -ne $expectedOutboxTests.Count -or $observed.Count -ne $expected.Count -or
        (Compare-Object -ReferenceObject $expected -DifferenceObject $observed -CaseSensitive)) {
        throw 'PHASE4_MANAGED_REPOSITORY_EXACT_TEST_SET_REJECTED'
    }
    $summaries = [regex]::Matches(
        $combined,
        '(?m)^test result: ok\. 6 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in [0-9.]+s$'
    )
    if ($summaries.Count -ne 1) {
        throw 'PHASE4_MANAGED_REPOSITORY_TEST_SUMMARY_REJECTED'
    }
    return [pscustomobject][ordered]@{
        tests = @($observed)
        passed = 6
        failed = 0
        ignored = 0
        measured = 0
        summary_sha256 = (
            [Convert]::ToHexString(
                [Security.Cryptography.SHA256]::HashData(
                    $utf8.GetBytes([string]$summaries[0].Value)
                )
            ).ToLowerInvariant()
        )
    }
}

function Start-OwnedPostgres {
    # Process creation can outlive or out-race the bounded pg_ctl launcher.
    # From this point onward, finalization must prove this exact data root is stopped.
    $script:postgresStartMayOwnProcess = $true
    $script:postgresLauncherTerminalProven = $false
    $launcher = Start-Phase4OwnedProcessJob -Executable $script:pgCtl -Argument @(
        '-D', $script:dataRoot, '-l', $script:logPath, '-o', $script:serverOptions,
        '-W', 'start'
    ) -Environment (Get-OutboxChildEnvironment -Additional ([ordered]@{})) `
        -WorkingDirectory $script:repositoryRoot `
        -Failure 'PHASE4_MANAGED_REPOSITORY_POSTGRES_START_FAILED' `
        -OutputEncodingCodePage $script:windowsOemCodePage
    $script:postgresOwnedProcessJob = $launcher
    $launcher.StandardInput.Close()
    if (-not $launcher.WaitForExit(30000)) {
        Stop-Phase4OwnedProcessJob -OwnedProcess $launcher `
            -Failure 'PHASE4_MANAGED_REPOSITORY_POSTGRES_START_TERMINATION_FAILED'
        $script:postgresLauncherTerminalProven = $true
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_START_TIMEOUT'
    }
    $script:postgresLauncherTerminalProven = $true
    if ($launcher.ExitCode -ne 0) {
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_START_FAILED'
    }
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(30)
    $ready = $false
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
    if (-not $ready) {
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_NOT_READY'
    }
    $script:postgresProcessIdentity = Get-OwnedPostgresProcessRecord
    if ($null -eq $script:postgresProcessIdentity.process) {
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_PROCESS_REJECTED'
    }
    $script:postgresRunning = $true
}

function Close-OutboxPostgresOwnedJob([switch]$TerminateRemaining) {
    if ($null -eq $script:postgresOwnedProcessJob) { return }
    $ownedJob = $script:postgresOwnedProcessJob
    try {
        if ([long]$ownedJob.ActiveProcessCount() -ne 0) {
            if (-not $TerminateRemaining) {
                throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
            }
            Stop-Phase4OwnedProcessJob -OwnedProcess $ownedJob `
                -Failure 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
        }
        Close-Phase4OwnedProcessJob -OwnedProcess $ownedJob `
            -Failure 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
    }
    finally {
        $ownedJob.Dispose()
        $script:postgresOwnedProcessJob = $null
    }
}

function Stop-OwnedPostgres {
    if (-not $script:postgresRunning -and -not $script:postgresStartMayOwnProcess) {
        return
    }
    if (-not (Test-Path -LiteralPath $script:markerPath -PathType Leaf)) {
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
    }
    try { $owner = [IO.File]::ReadAllText($script:markerPath, $utf8) | ConvertFrom-Json }
    catch { throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED' }
    if ([string]$owner.owner -cne $ownerKind -or [string]$owner.run_id -cne $script:runId -or
        [IO.Path]::GetFullPath([string]$owner.root) -cne [IO.Path]::GetFullPath($script:runRoot) -or
        [IO.Path]::GetFullPath([string]$owner.postgres) -cne
            [IO.Path]::GetFullPath($script:postgres) -or
        [string]$owner.postgres_sha256 -cne (Get-Sha256 $script:postgres)) {
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
    }
    $pidPath = Join-Path $script:dataRoot 'postmaster.pid'
    $absenceDeadline = [DateTimeOffset]::UtcNow.AddSeconds(5)
    $absenceObservations = 0
    while (-not (Test-Path -LiteralPath $pidPath -PathType Leaf) -and
        [DateTimeOffset]::UtcNow -lt $absenceDeadline) {
        $status = Invoke-OutboxOwnedProcess -Executable $script:pgCtl -Argument @(
            '-D', $script:dataRoot, 'status'
        ) -Environment (Get-OutboxChildEnvironment -Additional ([ordered]@{})) `
            -TimeoutSeconds 10 `
            -Failure 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED' `
            -OutputEncodingCodePage $script:windowsOemCodePage
        if (-not $script:postgresLauncherTerminalProven -or
            [int]$status.exit_code -ne 3 -or
            (Test-LoopbackPortOpen -Port $script:port)) {
            throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
        }
        $absenceObservations += 1
        Start-Sleep -Milliseconds 250
    }
    if (-not (Test-Path -LiteralPath $pidPath -PathType Leaf)) {
        if ($absenceObservations -lt 10) {
            throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
        }
        if ($null -ne $script:postgresOwnedProcessJob -and
            [long]$script:postgresOwnedProcessJob.ActiveProcessCount() -ne 0) {
            Close-OutboxPostgresOwnedJob -TerminateRemaining
        }
        if ($null -ne $script:postgresProcessIdentity) {
            Assert-OwnedProcessIdentityAbsent `
                -ProcessId ([int]$script:postgresProcessIdentity.process_id) `
                -ProcessStartUtcTicks ([long]$script:postgresProcessIdentity.process_start_utc_ticks) `
                -Failure 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
        }
        Close-OutboxPostgresOwnedJob
        $script:postgresRunning = $false
        $script:postgresStartMayOwnProcess = $false
        return
    }
    $owned = Get-OwnedPostgresProcessRecord
    if ($null -eq $owned.process) {
        if (Test-LoopbackPortOpen -Port $script:port) {
            throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
        }
        if ($null -ne $script:postgresOwnedProcessJob -and
            [long]$script:postgresOwnedProcessJob.ActiveProcessCount() -ne 0) {
            Close-OutboxPostgresOwnedJob -TerminateRemaining
        }
        if ($null -ne $script:postgresProcessIdentity) {
            Assert-OwnedProcessIdentityAbsent `
                -ProcessId ([int]$script:postgresProcessIdentity.process_id) `
                -ProcessStartUtcTicks ([long]$script:postgresProcessIdentity.process_start_utc_ticks) `
                -Failure 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
        }
        Close-OutboxPostgresOwnedJob
        $script:postgresRunning = $false
        $script:postgresStartMayOwnProcess = $false
        return
    }
    if ($null -ne $script:postgresProcessIdentity -and
        ([int]$owned.process_id -ne [int]$script:postgresProcessIdentity.process_id -or
         [long]$owned.process_start_utc_ticks -ne
            [long]$script:postgresProcessIdentity.process_start_utc_ticks)) {
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_OWNERSHIP_REJECTED'
    }
    try {
        $stop = Invoke-OutboxOwnedProcess -Executable $script:pgCtl -Argument @(
            '-D', $script:dataRoot, '-m', 'fast', '-w', 'stop'
        ) -Environment (Get-OutboxChildEnvironment -Additional ([ordered]@{})) `
            -TimeoutSeconds 60 -Failure 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_FAILED' `
            -OutputEncodingCodePage $script:windowsOemCodePage
    }
    catch {
        Close-OutboxPostgresOwnedJob -TerminateRemaining
        throw
    }
    if ([int]$stop.exit_code -ne 0) {
        Close-OutboxPostgresOwnedJob -TerminateRemaining
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_FAILED'
    }
    Assert-OwnedProcessIdentityAbsent -ProcessId ([int]$owned.process_id) `
        -ProcessStartUtcTicks ([long]$owned.process_start_utc_ticks) `
        -Failure 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_FAILED'
    if (Test-LoopbackPortOpen -Port $script:port) {
        throw 'PHASE4_MANAGED_REPOSITORY_POSTGRES_STOP_FAILED'
    }
    Close-OutboxPostgresOwnedJob
    $script:postgresRunning = $false
    $script:postgresStartMayOwnProcess = $false
}

foreach ($binary in @($initdb, $pgCtl, $postgres, $psql, $cargo, $node, $git)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw 'PHASE4_MANAGED_REPOSITORY_REQUIRED_BINARY_MISSING'
    }
}

$startedAt = [DateTimeOffset]::UtcNow
$runId = ([Guid]::NewGuid().ToString('N')).ToLowerInvariant()
$tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
    [IO.Path]::DirectorySeparatorChar
)
$rootName = 'lattice-phase4-managed-repository-' + $runId
$runRoot = [IO.Path]::GetFullPath((Join-Path $tempParent $rootName))
$expectedRoot = [IO.Path]::GetFullPath((Join-Path $tempParent $rootName))
$dataRoot = Join-Path $runRoot 'data'
$logPath = Join-Path $runRoot 'postgres.log'
$markerPath = Join-Path $runRoot '.managed-repository-owner.json'
$passwordPath = Join-Path $runRoot '.initdb-password'
$password = (([Guid]::NewGuid().ToString('N')) + ([Guid]::NewGuid().ToString('N'))).ToLowerInvariant()
$databaseName = 'lattice_task019_' + $runId.Substring(0, 8) + '_base'
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = [int]([Net.IPEndPoint]$listener.LocalEndpoint).Port
$listener.Stop()

$script:repositoryRoot = $repositoryRoot
$script:cargo = $cargo
$script:pgCtl = $pgCtl
$script:postgres = $postgres
$script:psql = $psql
$script:runRoot = $runRoot
$script:dataRoot = $dataRoot
$script:logPath = $logPath
$script:port = $port
$script:password = $password
$script:databaseName = $databaseName
$script:markerPath = $markerPath
$script:runId = $runId
$script:stageTimeoutSeconds = $StageTimeoutSeconds
$script:serverOptions = "-p $port -h 127.0.0.1 -c ssl=off -c fsync=on " +
    '-c synchronous_commit=on -c full_page_writes=on -c max_prepared_transactions=0'
$script:postgresRunning = $false
$script:postgresStartMayOwnProcess = $false
$script:postgresLauncherTerminalProven = $false
$script:postgresProcessIdentity = $null
$script:postgresOwnedProcessJob = $null
$script:currentStage = 'PREPARE'
$script:stages = [Collections.Generic.List[string]]::new()
$script:stageEvidence = [Collections.Generic.List[object]]::new()
$script:providerDispatchSnapshots = [Collections.Generic.List[object]]::new()
$cleanupFailures = [Collections.Generic.List[string]]::new()
$result = $null
$exitCode = 0

try {
    if ($runRoot -cne $expectedRoot -or (Test-Path -LiteralPath $runRoot) -or
        -not $runRoot.StartsWith(
            $tempParent + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase
        )) {
        throw 'PHASE4_MANAGED_REPOSITORY_ROOT_REJECTED'
    }
    [IO.Directory]::CreateDirectory($dataRoot) | Out-Null
    $marker = [ordered]@{
        owner = $ownerKind
        run_id = $runId
        root = $runRoot
        postgres = [IO.Path]::GetFullPath($postgres)
        postgres_sha256 = Get-Sha256 $postgres
    }
    [IO.File]::WriteAllText($markerPath, ($marker | ConvertTo-Json -Compress), $utf8)
    [IO.File]::WriteAllText($passwordPath, $password, [Text.Encoding]::ASCII)
    $init = Invoke-OutboxOwnedProcess -Executable $initdb -Argument @(
        '-D', $dataRoot, '-U', 'runtime_bootstrap', '--auth-host=scram-sha-256',
        '--auth-local=trust', '--encoding=UTF8', '--locale=C', '--data-checksums',
        ('--pwfile=' + $passwordPath)
    ) -Environment (Get-OutboxChildEnvironment -Additional ([ordered]@{})) `
        -TimeoutSeconds 60 -Failure 'PHASE4_MANAGED_REPOSITORY_INITDB_FAILED' `
        -OutputEncodingCodePage $script:windowsOemCodePage
    if ([int]$init.exit_code -ne 0) {
        throw 'PHASE4_MANAGED_REPOSITORY_INITDB_FAILED'
    }
    Remove-Item -LiteralPath $passwordPath -Force
    if (Test-Path -LiteralPath $passwordPath) {
        throw 'PHASE4_MANAGED_REPOSITORY_PASSWORD_CLEANUP_FAILED'
    }
    Start-OwnedPostgres

    $env:LATTICE_TASK019_HOST = '127.0.0.1'
    $env:LATTICE_TASK019_PORT = [string]$port
    $env:LATTICE_TASK019_RUN_ID = $runId
    $env:LATTICE_TASK019_PASSWORD = $password
    $env:LATTICE_FULL_CHAIN_RUN_MODE = 'RESUME_EXISTING'
    $env:LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
    $env:LATTICE_DELIVERY_TIMEOUT_SECONDS = '120'
    $env:LATTICE_STORE_DAEMON_INSTANCE_ID = 'task050-fresh-process'
    $env:LATTICE_STORE_DAEMON_EPOCH = '50'
    $env:LATTICE_STORE_AUTHORITY_REVISION = '50'
    $env:LATTICE_STORE_OBSERVATION_DIGEST = ('a' * 64)
    $env:LATTICE_STORE_AUTHORITY_HEAD_DIGEST = ('b' * 64)
    $env:LATTICE_MANAGED_FOREMAN_MODE = 'DISABLED'
    Invoke-CargoStage 'PRODUCT_POSTGRES_INITIALIZE' @(
        'run', '-p', 'lattice-runtime', '--bin', 'latticed', '--locked', '--offline',
        '--', '--postgres-initialize'
    )
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

    $env:LATTICE_MANAGED_REPOSITORY_LIVE = '1'
    $env:LATTICE_MANAGED_REPOSITORY_PORT = [string]$port
    $env:LATTICE_MANAGED_REPOSITORY_RUN_ID = $runId
    $env:LATTICE_MANAGED_REPOSITORY_PASSWORD = $password
    $env:LATTICE_MANAGED_REPOSITORY_ROOT = $runRoot
    $env:LATTICE_MANAGED_REPOSITORY_NODE = $node
    $env:LATTICE_MANAGED_REPOSITORY_GIT = $git
    $providerDispatchBefore = Get-ManagedRepositoryProviderDispatchCount `
        -Phase 'BEFORE_OUTBOX_CRASH_WINDOWS'
    Invoke-CargoStage 'MANAGED_REPOSITORY_OUTBOX_CRASH_WINDOWS' @(
        'test', '-p', 'lattice-runtime', '--lib', '--locked', '--offline',
        'postgres_repository_', '--', '--ignored', '--nocapture', '--test-threads=1'
    )
    $outboxStageEvidence = @($script:stageEvidence | Where-Object {
        [string]$_.stage -ceq 'MANAGED_REPOSITORY_OUTBOX_CRASH_WINDOWS'
    })
    if ($outboxStageEvidence.Count -ne 1) {
        throw 'PHASE4_MANAGED_REPOSITORY_TEST_EVIDENCE_REJECTED'
    }
    $exactCargoEvidence = Assert-ManagedRepositoryOutboxCargoEvidence `
        -Evidence $outboxStageEvidence[0]
    $providerDispatchAfter = Get-ManagedRepositoryProviderDispatchCount `
        -Phase 'AFTER_OUTBOX_CRASH_WINDOWS'
    if ($providerDispatchBefore -ne 0 -or $providerDispatchAfter -ne 0) {
        throw 'PHASE4_MANAGED_REPOSITORY_PROVIDER_EFFECT_CHANGED'
    }

    $result = [ordered]@{
        schema = 'lattice.phase4-managed-repository-outbox-live.v1'
        status = 'PASS'
        run_id = $runId
        database = $databaseName
        port = $port
        tests = @($exactCargoEvidence.tests)
        exact_test_summary = $exactCargoEvidence
        provider_dispatch_snapshots = @($script:providerDispatchSnapshots)
        provider_dispatch_before_observed = $providerDispatchBefore
        provider_dispatch_after_observed = $providerDispatchAfter
        stage_evidence = @($script:stageEvidence)
        stages = @($script:stages)
        artifacts_retained = $true
        evidence_root = $runRoot
    }
}
catch {
    $exitCode = 1
    $failure = if ($_.Exception.Message -cmatch '\A[A-Z][A-Z0-9_]{0,127}\z') {
        $_.Exception.Message
    }
    else {
        'PHASE4_MANAGED_REPOSITORY_STAGE_FAILED_' + $script:currentStage
    }
    $result = [ordered]@{
        schema = 'lattice.phase4-managed-repository-outbox-live.v1'
        status = 'FAIL'
        failure = $failure
        completed_stages = @($script:stages)
        evidence_root = $runRoot
    }
}
finally {
    try {
        Stop-OwnedPostgres
    }
    catch {
        $exitCode = 1
        $cleanupFailures.Add('PHASE4_MANAGED_REPOSITORY_FINAL_STOP_FAILED')
        if ($null -ne $script:postgresOwnedProcessJob) {
            try { Close-OutboxPostgresOwnedJob -TerminateRemaining }
            catch {
                $cleanupFailures.Add(
                    'PHASE4_MANAGED_REPOSITORY_FINAL_PROCESS_TREE_STOP_FAILED'
                )
            }
        }
        if ($null -ne $result -and $result.status -ceq 'PASS') {
            $result['status'] = 'FAIL'
            $result['failure'] = 'PHASE4_MANAGED_REPOSITORY_FINAL_STOP_FAILED'
            $result['evidence_root'] = $runRoot
        }
    }

    try {
        if (Test-Path -LiteralPath $passwordPath) {
            Remove-Item -LiteralPath $passwordPath -Force
        }
        if (Test-Path -LiteralPath $passwordPath) {
            throw 'PHASE4_MANAGED_REPOSITORY_PASSWORD_CLEANUP_FAILED'
        }
    }
    catch {
        $exitCode = 1
        $cleanupFailures.Add('PHASE4_MANAGED_REPOSITORY_PASSWORD_CLEANUP_FAILED')
        if ($null -ne $result -and $result.status -ceq 'PASS') {
            $result['status'] = 'FAIL'
            $result['failure'] = 'PHASE4_MANAGED_REPOSITORY_PASSWORD_CLEANUP_FAILED'
            $result['evidence_root'] = $runRoot
        }
    }

    foreach ($name in @(
        'LATTICE_TASK019_PASSWORD',
        'LATTICE_MANAGED_REPOSITORY_PASSWORD',
        'LATTICE_MANAGED_REPOSITORY_LIVE',
        'LATTICE_MANAGED_REPOSITORY_PORT',
        'LATTICE_MANAGED_REPOSITORY_RUN_ID',
        'LATTICE_MANAGED_REPOSITORY_ROOT',
        'LATTICE_MANAGED_REPOSITORY_NODE',
        'LATTICE_MANAGED_REPOSITORY_GIT'
    )) {
        Remove-Item -LiteralPath ('Env:' + $name) -ErrorAction SilentlyContinue
    }

    if ($null -ne $result -and $result.status -ceq 'PASS') {
        $result['artifacts_retained'] = $true
        $result['evidence_root'] = $runRoot
    }
    if ($null -ne $result) {
        $result['cleanup_failures'] = @($cleanupFailures)
    }
}

$finishedAt = [DateTimeOffset]::UtcNow
$result['started_at'] = $startedAt.ToString('O')
$result['finished_at'] = $finishedAt.ToString('O')
$result['duration_seconds'] = [Math]::Round(($finishedAt - $startedAt).TotalSeconds, 3)
$result | ConvertTo-Json -Depth 8 -Compress
exit $exitCode
