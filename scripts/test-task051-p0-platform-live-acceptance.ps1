[CmdletBinding()]
param(
    [switch]$SelfTestOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$runner = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'run-task051-p0-platform-live-acceptance.ps1'))
$runnerItem = Get-Item -LiteralPath $runner -Force -ErrorAction SilentlyContinue
if (
    $null -eq $runnerItem -or
    $runnerItem.PSIsContainer -or
    -not ($runnerItem -is [IO.FileInfo]) -or
    ($runnerItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
) {
    throw 'TASK051_RUNNER_REJECTED'
}

$tokens = $null
$parseErrors = $null
[void][Management.Automation.Language.Parser]::ParseFile(
    $runner,
    [ref]$tokens,
    [ref]$parseErrors
)
if (@($parseErrors).Count -ne 0) {
    throw 'TASK051_RUNNER_PARSE_REJECTED'
}

$runnerSource = [IO.File]::ReadAllText($runner)
$requiredFragments = @(
    '[switch]$LibraryOnly',
    '[switch]$SelfTestOnly',
    'task051-p0-platform-live-acceptance',
    'Assert-Task051NoReparseAncestor',
    'TASK051_ALLOWED_ROOT_REPARSE_REJECTED',
    'TASK051_RUN_ROOT_REPARSE_REJECTED',
    'TASK050_FULLY_VERIFIED',
    '8e5ba40d38b781afff7028841bd981c8dd2b9721',
    'mcpServerStatus/list',
    'app-server',
    '--stdio',
    '--ephemeral',
    '--json',
    'lattice_delivery_run',
    'lattice_delivery_status',
    'lattice_task_submit',
    'lattice_task_status',
    'Invoke-Task051CodexTool',
    'lattice.task.status.v1',
    'New-Task051OwnerOnlyDirectory',
    'Start-Task038SuspendedProcess',
    'Add-Task038ProcessToJob',
    'Stop-Task038Job',
    'Close-Task038Job',
    'Stop-Task038ProcessTree',
    'TASK051_PROCESS_START_CLEANUP_REJECTED',
    'Initialize-Task051CargoHome',
    'CARGO_HOME',
    'CARGO_NET_OFFLINE',
    'TASK051_CARGO_HOME_CLEANUP_REJECTED',
    'CARGO_TARGET_DIR',
    'TEMP',
    'TMP',
    'TASK051_TEMP_CLEANUP_REJECTED',
    'New-Task051RunRootAlias',
    'Remove-Task051RunRootAlias',
    'TASK051_RUN_ALIAS_CLEANUP_REJECTED',
    'LATTICE_TASK051_RUN_ALIAS_ROOT',
    'TASK051_TASK019_PGDATA_ALIAS_TRANSFORM_REJECTED',
    'TASK051_TASK019_INITDB_ALIAS_TRANSFORM_REJECTED',
    "PSObject.Properties['outputSchema']",
    'lattice.task051.current-codex-discovery.v1',
    'lattice.task051.current-codex-tool-call.v1',
    'TASK051_CODEX_SUBMIT_CALL_COUNT_REJECTED',
    'TASK051_CODEX_STATUS_CALL_COUNT_REJECTED',
    'collab_tool_call',
    'TASK051_CODEX_UNEXPECTED_TOOL_REJECTED',
    'TASK051_CODEX_FRESH_PROCESS_REJECTED',
    'V5_MEMORY_V3_WRITER_LEASE_V2',
    'CONTROLLED_CODEX_CANARY_AUTONOMY_V1',
    'task_ledger_autonomy_receipts',
    'TASK051_AUTONOMY_RECEIPT_REJECTED',
    'Restart-DisposablePostgres',
    'TASK051_CONFIG_ROLLBACK_REJECTED',
    'TASK051_P0_PLATFORM_LIVE_ACCEPTANCE=PASS'
)
foreach ($fragment in $requiredFragments) {
    if ($runnerSource.IndexOf($fragment, [StringComparison]::Ordinal) -lt 0) {
        throw ('TASK051_RUNNER_SHAPE_REJECTED|' + $fragment)
    }
}

$forbiddenFragments = @(
    '[IO.File]::WriteAllBytes($originalConfig',
    '[IO.File]::WriteAllText($originalConfig',
    'Set-Content -LiteralPath $originalConfig',
    'Add-Content -LiteralPath $originalConfig',
    'git push --force',
    'codex mcp add',
    'codex mcp login',
    'git clean',
    'git reset --hard'
)
foreach ($fragment in $forbiddenFragments) {
    if ($runnerSource.IndexOf($fragment, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
        throw ('TASK051_RUNNER_FORBIDDEN_SHAPE|' + $fragment)
    }
}

$selfTestOutput = @(& $runner -SelfTestOnly 2>&1 | ForEach-Object { [string]$_ })
$selfTestSucceeded = $?
if (-not $selfTestSucceeded) {
    throw 'TASK051_RUNNER_SELF_TEST_REJECTED'
}
$expectedSelfTestMarkers = @(
    'TASK051_SOURCE_TRANSFORM_SELF_TEST=PASS',
    'TASK051_CODEX_EVENT_PARSER_SELF_TEST=PASS',
    'TASK051_APP_SERVER_DISCOVERY_SELF_TEST=PASS',
    'TASK051_OWNER_ONLY_CREDENTIAL_SELF_TEST=PASS',
    'TASK051_PROCESS_CONTAINMENT_SELF_TEST=PASS',
    'TASK051_RUNNER_SELF_TEST=PASS'
)
foreach ($marker in $expectedSelfTestMarkers) {
    if (@($selfTestOutput | Where-Object { $_ -ceq $marker }).Count -ne 1) {
        throw ('TASK051_RUNNER_SELF_TEST_MARKER_REJECTED|' + $marker)
    }
}

if ($SelfTestOnly) {
    Write-Output 'TASK051_WRAPPER_SELF_TEST=PASS'
    return
}

& $runner
$runnerSucceeded = $?
if (-not $runnerSucceeded) {
    throw 'TASK051_RUNNER_EXECUTION_REJECTED'
}

Write-Output 'TASK051_P0_PLATFORM_LIVE_ACCEPTANCE=PASS'
