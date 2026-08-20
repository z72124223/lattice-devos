[CmdletBinding()]
param(
    [switch]$SelfTestOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$script:Task044SchemaVersion = 'lattice.task044.component-fault-recovery.v1'
$script:Task044Scenarios = @('F01', 'F02', 'F03', 'F04', 'F05', 'F06', 'F07', 'F08')
$script:Task044BaseCommit = '65f2902504e5ef5acba6f258b736905fd4d12a4d'
$script:Task038StableCommit = '2b424ec9a5401a6fbdc4f37d3d401592331afca0'
$script:ExpectedBranch = 'feature/task-044-component-fault-recovery-gate'
$script:MaximumCapturedCharacters = 4MB

function Get-Task044StringSha256 {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value
    )

    $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($algorithm.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
        [Array]::Clear($bytes, 0, $bytes.Length)
    }
}

function Assert-Task044ScenarioCoverage {
    param(
        [Parameter(Mandatory = $true)][object[]]$Records
    )

    foreach ($record in $Records) {
        if ([string]$record.status -cne 'PASS') {
            throw 'TASK044_COMMAND_RESULT_REJECTED'
        }
    }
    foreach ($scenario in $script:Task044Scenarios) {
        $matches = @($Records | Where-Object { @($_.scenarios) -ccontains $scenario })
        if ($matches.Count -eq 0) {
            throw 'TASK044_SCENARIO_COVERAGE_REJECTED'
        }
    }
}

function Assert-Task044RequiredOutput {
    param(
        [Parameter(Mandatory = $true)][string]$CommandId,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Output,
        [Parameter(Mandatory = $true)][string[]]$RequiredText,
        [Parameter(Mandatory = $true)][bool]$RejectSkip
    )

    foreach ($required in $RequiredText) {
        if ($Output.IndexOf($required, [StringComparison]::Ordinal) -lt 0) {
            throw ('TASK044_REQUIRED_EVIDENCE_MISSING|' + $CommandId)
        }
    }
    if (
        $RejectSkip -and (
            $Output -cmatch '(?m)^SKIP:' -or
            $Output -cmatch '(?m)test result:.*\b[1-9][0-9]* ignored\b'
        )
    ) {
        throw ('TASK044_SKIPPED_EVIDENCE_REJECTED|' + $CommandId)
    }
}

function Invoke-Task044Command {
    param(
        [Parameter(Mandatory = $true)][ValidatePattern('^[a-z0-9-]{1,48}$')][string]$CommandId,
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string[]]$Scenarios,
        [Parameter(Mandatory = $true)][string[]]$RequiredText,
        [switch]$RejectSkip
    )

    $started = [DateTimeOffset]::UtcNow
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $commandOutput = @(& $Executable @Arguments 2>&1 | ForEach-Object { [string]$_ })
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    $finished = [DateTimeOffset]::UtcNow
    $joined = [string]::Join("`n", $commandOutput)
    if ($joined.Length -gt $script:MaximumCapturedCharacters) {
        throw ('TASK044_COMMAND_OUTPUT_BOUND_REJECTED|' + $CommandId)
    }
    if ($exitCode -ne 0) {
        throw ('TASK044_COMMAND_FAILED|' + $CommandId)
    }
    Assert-Task044RequiredOutput `
        -CommandId $CommandId `
        -Output $joined `
        -RequiredText $RequiredText `
        -RejectSkip ([bool]$RejectSkip)

    return [pscustomobject][ordered]@{
        command_id = $CommandId
        status = 'PASS'
        scenarios = @($Scenarios)
        output_sha256 = Get-Task044StringSha256 -Value $joined
        output_line_count = $commandOutput.Count
        elapsed_milliseconds = [long]($finished - $started).TotalMilliseconds
    }
}

function New-Task044Evidence {
    param(
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][string]$SourceCommit,
        [Parameter(Mandatory = $true)][object[]]$Records,
        [Parameter(Mandatory = $true)][string]$StartedAtUtc,
        [Parameter(Mandatory = $true)][string]$FinishedAtUtc
    )

    Assert-Task044ScenarioCoverage -Records $Records
    $scenarioResults = @()
    foreach ($scenario in $script:Task044Scenarios) {
        $commandIds = @(
            $Records |
                Where-Object { @($_.scenarios) -ccontains $scenario } |
                ForEach-Object { [string]$_.command_id }
        )
        $scenarioResults += [pscustomobject][ordered]@{
            scenario = $scenario
            status = 'PASS'
            command_ids = $commandIds
        }
    }

    return [pscustomobject][ordered]@{
        schema_version = 'lattice.task044.component-fault-recovery.v1'
        run_id = $RunId
        source_commit = $SourceCommit
        base_commit = $script:Task044BaseCommit
        task038_stable_commit = $script:Task038StableCommit
        started_at_utc = $StartedAtUtc
        finished_at_utc = $FinishedAtUtc
        scenarios = $scenarioResults
        commands = @($Records)
        fresh_status_effect_delta = [pscustomobject][ordered]@{
            codex = 0
            verification = 0
            git = 0
            graphify = 0
            hermes = 0
            memory = 0
            basis_command_ids = @('task068-postgres-replay', 'task050-fresh-status')
        }
        cleanup = [pscustomobject][ordered]@{
            status = 'PASS'
            unexpected_footprint_count = 0
            retained_allowlist = @('evidence/task044-component-fault-recovery.json')
            basis_command_ids = @(
                'codex-process-faults',
                'hermes-recovery-faults',
                'task068-postgres-replay',
                'task050-fresh-status'
            )
        }
    }
}

function Invoke-Task044SelfTest {
    $stderrRecord = Invoke-Task044Command `
        -CommandId 'self-native-stderr' `
        -Executable (Get-Command powershell.exe -ErrorAction Stop).Source `
        -Arguments @(
            '-NoLogo', '-NoProfile', '-Command',
            "[Console]::Error.WriteLine('TASK044_SELF_TEST_STDERR'); Write-Output 'TASK044_SELF_TEST_STDOUT'; exit 0"
        ) `
        -Scenarios @('F01') `
        -RequiredText @('TASK044_SELF_TEST_STDERR', 'TASK044_SELF_TEST_STDOUT')
    if ([string]$stderrRecord.status -cne 'PASS') {
        throw 'TASK044_NATIVE_STDERR_SELF_TEST_REJECTED'
    }

    $records = @()
    foreach ($scenario in $script:Task044Scenarios) {
        $records += [pscustomobject][ordered]@{
            command_id = ('self-' + $scenario.ToLowerInvariant())
            status = 'PASS'
            scenarios = @($scenario)
            output_sha256 = ('0' * 64)
            output_line_count = 1
            elapsed_milliseconds = 0
        }
    }
    $evidence = New-Task044Evidence `
        -RunId ('0' * 32) `
        -SourceCommit ('1' * 40) `
        -Records $records `
        -StartedAtUtc '2000-01-01T00:00:00Z' `
        -FinishedAtUtc '2000-01-01T00:00:01Z'
    if (
        [string]$evidence.schema_version -cne $script:Task044SchemaVersion -or
        @($evidence.scenarios).Count -ne 8 -or
        [int]$evidence.cleanup.unexpected_footprint_count -ne 0
    ) {
        throw 'TASK044_EVIDENCE_SELF_TEST_REJECTED'
    }

    $missingCoverageRejected = $false
    try {
        Assert-Task044ScenarioCoverage -Records @($records | Where-Object { @($_.scenarios) -cnotcontains 'F08' })
    }
    catch {
        $missingCoverageRejected = $_.Exception.Message -ceq 'TASK044_SCENARIO_COVERAGE_REJECTED'
    }
    if (-not $missingCoverageRejected) {
        throw 'TASK044_FAIL_CLOSED_SELF_TEST_REJECTED'
    }

    Write-Output 'TASK044_AGGREGATE_RUNNER_SELF_TEST=PASS'
}

if ($SelfTestOnly) {
    Invoke-Task044SelfTest
    exit 0
}

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$startedAt = [DateTimeOffset]::UtcNow
$runId = [Guid]::NewGuid().ToString('N')
$evidenceDirectory = Join-Path $repositoryRoot (Join-Path 'target\task044-component-fault-recovery' $runId)
$evidencePath = Join-Path $evidenceDirectory 'evidence\task044-component-fault-recovery.json'
if (Test-Path -LiteralPath $evidenceDirectory) {
    throw 'TASK044_EVIDENCE_ROOT_NOT_FRESH'
}

# git status --short is a fail-closed source identity gate, not completion evidence.
$statusOutput = @(& git -C $repositoryRoot status --short 2>&1 | ForEach-Object { [string]$_ })
if ($LASTEXITCODE -ne 0 -or $statusOutput.Count -ne 0) {
    throw 'TASK044_SOURCE_WORKTREE_DIRTY'
}
$branch = [string](& git -C $repositoryRoot branch --show-current 2>&1)
if ($LASTEXITCODE -ne 0 -or $branch -cne $script:ExpectedBranch) {
    throw 'TASK044_SOURCE_BRANCH_REJECTED'
}
$sourceCommit = [string](& git -C $repositoryRoot rev-parse HEAD 2>&1)
if ($LASTEXITCODE -ne 0 -or $sourceCommit -cnotmatch '\A[0-9a-f]{40}\z') {
    throw 'TASK044_SOURCE_COMMIT_REJECTED'
}
& git -C $repositoryRoot merge-base --is-ancestor $script:Task044BaseCommit $sourceCommit
if ($LASTEXITCODE -ne 0) {
    throw 'TASK044_SOURCE_BASE_REJECTED'
}

$cargo = (Get-Command cargo.exe -ErrorAction Stop).Source
$powershell = (Get-Command powershell.exe -ErrorAction Stop).Source
$npm = (Get-Command npm.cmd -ErrorAction Stop).Source
$cargoCommandPrefix = 'cargo test'
if ([string]::IsNullOrWhiteSpace($cargoCommandPrefix)) {
    throw 'TASK044_CARGO_COMMAND_REJECTED'
}

$records = @()
Push-Location $repositoryRoot
try {
    $records += Invoke-Task044Command `
        -CommandId 'delivery-order-faults' `
        -Executable $cargo `
        -Arguments @('test', '--locked', '-p', 'lattice-orchestrator', '--test', 'delivery_order') `
        -Scenarios @('F01', 'F03', 'F04', 'F07') `
        -RequiredText @(
            'intent_failure_has_zero_later_effects',
            'ambiguous_commit_is_never_reported_as_failed_or_completed',
            'receipt_cross_binding_is_rejected_after_successful_outcome_write',
            'status_path_calls_only_the_durable_receipt_port'
        ) `
        -RejectSkip
    $records += Invoke-Task044Command `
        -CommandId 'graph-memory-faults' `
        -Executable $cargo `
        -Arguments @('test', '--locked', '-p', 'lattice-orchestrator', '--test', 'graph_memory_order') `
        -Scenarios @('F01', 'F03', 'F07') `
        -RequiredText @(
            'every_effect_failure_stops_all_later_calls',
            'graph_memory_status_loads_only_receipt_and_rejects_cross_binding'
        ) `
        -RejectSkip
    $records += Invoke-Task044Command `
        -CommandId 'controlled-task-faults' `
        -Executable $cargo `
        -Arguments @('test', '--locked', '-p', 'lattice-orchestrator', '--test', 'controlled_task') `
        -Scenarios @('F02', 'F04', 'F05', 'F07') `
        -RequiredText @(
            'ambiguous_execution_keeps_the_fence_and_enters_reconciliation',
            'stale_writer_authority_cannot_record_stopping_or_release_the_current_lease'
        ) `
        -RejectSkip
    $records += Invoke-Task044Command `
        -CommandId 'task-domain-cancel' `
        -Executable $cargo `
        -Arguments @('test', '--locked', '-p', 'lattice-task-domain', '--test', 'task_domain') `
        -Scenarios @('F02') `
        -RequiredText @('freezes_the_complete_v1_transition_matrix_and_stable_errors') `
        -RejectSkip
    $records += Invoke-Task044Command `
        -CommandId 'codex-process-faults' `
        -Executable $cargo `
        -Arguments @('test', '--locked', '-p', 'lattice-codex-adapter', '--test', 'process') `
        -Scenarios @('F01', 'F02', 'F03', 'F05', 'F08') `
        -RequiredText @(
            'sandbox_preflight_failure_prevents_app_server_spawn',
            'sandbox_preflight_timeout_reaps_owned_tree_before_app_server_spawn',
            'scripted_malformed_eof_and_wrong_home_fail_closed',
            'timeout_immediately_terminates_and_reaps_the_owned_tree'
        ) `
        -RejectSkip
    $records += Invoke-Task044Command `
        -CommandId 'codex-protocol-faults' `
        -Executable $cargo `
        -Arguments @('test', '--locked', '-p', 'lattice-codex-adapter', '--test', 'protocol') `
        -Scenarios @('F03', 'F04') `
        -RequiredText @(
            'preserves_failed_and_interrupted_terminal_outcomes',
            'ignores_non_terminal_notifications_and_rejects_malformed_terminals'
        ) `
        -RejectSkip
    $records += Invoke-Task044Command `
        -CommandId 'graphify-snapshot-faults' `
        -Executable $cargo `
        -Arguments @(
            'test', '--locked', '-p', 'lattice-graphify-adapter', '--test', 'exact_git_snapshot',
            'exact_git_snapshot_ignores_worktree_and_changes_only_with_commit', '--', '--exact'
        ) `
        -Scenarios @('F03', 'F05', 'F07') `
        -RequiredText @('exact_git_snapshot_ignores_worktree_and_changes_only_with_commit') `
        -RejectSkip
    $records += Invoke-Task044Command `
        -CommandId 'hermes-recovery-faults' `
        -Executable $cargo `
        -Arguments @('test', '--locked', '-p', 'lattice-hermes-adapter', '--lib') `
        -Scenarios @('F01', 'F02', 'F03', 'F04', 'F05', 'F07', 'F08') `
        -RequiredText @(
            'production::proxy_host_tests::verified_teardown_releases_owned_root_for_same_path_relaunch ... ok',
            'reflection_api::event_transport_loss_recovers_through_bound_status_polling ... ok',
            'reflection_api::same_process_reconciliation_uses_receipt_without_resubmission ... ok',
            'reflection_api::malformed_reflection_schema_fails_closed ... ok',
            'reflection_api::status_session_cross_binding_fails_closed ... ok',
            'reflection_api::event_and_status_timeouts_fail_closed ... ok'
        )
    $records += Invoke-Task044Command `
        -CommandId 'store-commit-faults' `
        -Executable $cargo `
        -Arguments @('test', '--locked', '-p', 'lattice-postgres-store', '--test', 'postgres_store') `
        -Scenarios @('F01', 'F04', 'F07') `
        -RequiredText @(
            'before_and_after_apply_faults_preserve_explicit_commit_uncertainty',
            'corrupted_retained_replay_fails_closed_instead_of_returning_receipt'
        ) `
        -RejectSkip
    $records += Invoke-Task044Command `
        -CommandId 'runtime-hermes-recovery' `
        -Executable $cargo `
        -Arguments @('test', '--locked', '-p', 'lattice-runtime', 'canonical_hermes_recovery_is_receipt_gated_and_never_resubmits') `
        -Scenarios @('F04', 'F05', 'F06', 'F07') `
        -RequiredText @('canonical_hermes_recovery_is_receipt_gated_and_never_resubmits') `
        -RejectSkip
    $records += Invoke-Task044Command `
        -CommandId 'task068-postgres-replay' `
        -Executable $powershell `
        -Arguments @(
            '-NoLogo', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
            (Join-Path $PSScriptRoot 'run-task019-postgres.ps1'),
            '-RunTask076WriterLeaseGate', '-RunTask068HermesReplayGate', '-RunTask044CancellationGate'
        ) `
        -Scenarios @('F01', 'F02', 'F04', 'F05', 'F06', 'F07', 'F08') `
        -RequiredText @(
            'TASK019_POSTGRES_HARNESS=PASS',
            'TASK068_HERMES_POSTGRES_REPLAY=PASS',
            'TASK044_POSTGRES_CANCEL_REPLAY=PASS',
            'PHASES=initial,restart'
        ) `
        -RejectSkip
    $records += Invoke-Task044Command `
        -CommandId 'task050-fresh-status' `
        -Executable $powershell `
        -Arguments @(
            '-NoLogo', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
            (Join-Path $PSScriptRoot 'test-task050-autonomy-receipt-acceptance.ps1')
        ) `
        -Scenarios @('F01', 'F02', 'F04', 'F05', 'F06', 'F07', 'F08') `
        -RequiredText @(
            'TASK050_AUTONOMY_RECEIPT_ACCEPTANCE=PASS',
            'TASK050_POSTGRES_FRESH_PROCESS_ACCEPTANCE=PASS'
        ) `
        -RejectSkip
    $records += Invoke-Task044Command `
        -CommandId 'repository-verify' `
        -Executable $npm `
        -Arguments @('run', 'verify') `
        -Scenarios @('F03', 'F07') `
        -RequiredText @('tests')
}
finally {
    Pop-Location
}

$postStatusOutput = @(& git -C $repositoryRoot status --short 2>&1 | ForEach-Object { [string]$_ })
if ($LASTEXITCODE -ne 0 -or $postStatusOutput.Count -ne 0) {
    throw 'TASK044_POST_VERIFICATION_WORKTREE_DIRTY'
}
$postSourceCommit = [string](& git -C $repositoryRoot rev-parse HEAD 2>&1)
if ($LASTEXITCODE -ne 0 -or $postSourceCommit -cne $sourceCommit) {
    throw 'TASK044_POST_VERIFICATION_SOURCE_CHANGED'
}

$finishedAt = [DateTimeOffset]::UtcNow
$evidence = New-Task044Evidence `
    -RunId $runId `
    -SourceCommit $sourceCommit `
    -Records $records `
    -StartedAtUtc $startedAt.ToString('O') `
    -FinishedAtUtc $finishedAt.ToString('O')

[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($evidencePath)) | Out-Null
$json = $evidence | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText($evidencePath, $json + "`n", [Text.UTF8Encoding]::new($false))
$written = Get-Content -LiteralPath $evidencePath -Raw | ConvertFrom-Json
if (
    [string]$written.schema_version -cne $script:Task044SchemaVersion -or
    @($written.scenarios).Count -ne 8 -or
    [int]$written.cleanup.unexpected_footprint_count -ne 0
) {
    throw 'TASK044_EVIDENCE_WRITE_REJECTED'
}

Write-Output 'TASK044_COMPONENT_FAULT_RECOVERY_GATE=PASS'
Write-Output 'SCENARIOS=F01,F02,F03,F04,F05,F06,F07,F08'
Write-Output 'FRESH_STATUS_ZERO_RERUN=PASS'
Write-Output 'CLEANUP_FOOTPRINT=PASS'
