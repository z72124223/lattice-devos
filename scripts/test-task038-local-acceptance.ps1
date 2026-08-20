[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$harness = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'run-task038-task-submit.ps1'))
$item = Get-Item -LiteralPath $harness -Force -ErrorAction SilentlyContinue
if (
    $null -eq $item -or
    $item.PSIsContainer -or
    -not ($item -is [IO.FileInfo]) -or
    ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)
) {
    throw 'TASK038_LOCAL_HARNESS_REJECTED'
}

$tokens = $null
$parseErrors = $null
$scriptAst = [Management.Automation.Language.Parser]::ParseFile(
    $harness,
    [ref]$tokens,
    [ref]$parseErrors
)
if (@($parseErrors).Count -ne 0) {
    throw 'TASK038_LOCAL_HARNESS_PARSE_REJECTED'
}

$text = [IO.File]::ReadAllText($harness)
$strictUtf8 = [Text.UTF8Encoding]::new($false, $true)
function Read-Task038CandidateSource {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $candidatePath = [IO.Path]::GetFullPath($Path)
    $candidateItem = Get-Item -LiteralPath $candidatePath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $candidateItem -or
        $candidateItem.PSIsContainer -or
        -not ($candidateItem -is [IO.FileInfo]) -or
        ($candidateItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        -not [string]::Equals($candidateItem.FullName, $candidatePath, [StringComparison]::OrdinalIgnoreCase)
    ) {
        throw ($FailureCode + '_SOURCE_REJECTED')
    }
    $bytes = [IO.File]::ReadAllBytes($candidatePath)
    if (
        $bytes.Length -ge 3 -and
        $bytes[0] -eq 0xEF -and
        $bytes[1] -eq 0xBB -and
        $bytes[2] -eq 0xBF
    ) {
        throw ($FailureCode + '_UTF8_BOM_REJECTED')
    }
    try {
        $source = $strictUtf8.GetString($bytes)
    }
    catch {
        throw ($FailureCode + '_UTF8_REJECTED')
    }
    $candidateTokens = $null
    $candidateParseErrors = $null
    [void][Management.Automation.Language.Parser]::ParseFile(
        $candidatePath,
        [ref]$candidateTokens,
        [ref]$candidateParseErrors
    )
    if (@($candidateParseErrors).Count -ne 0) {
        throw ($FailureCode + '_PARSE_REJECTED')
    }
    return $source
}

$tunnelLauncher = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'start-chatgpt-mcp-tunnel.ps1'))
$tunnelLauncherText = Read-Task038CandidateSource `
    -Path $tunnelLauncher `
    -FailureCode 'TASK038_TUNNEL_LIFECYCLE'
$postgresHarness = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'run-task019-postgres.ps1'))
$postgresHarnessText = Read-Task038CandidateSource `
    -Path $postgresHarness `
    -FailureCode 'TASK038_TUNNEL_POSTGRES_HOOK'

$lifecycleMaterializationFragments = @(
    'TUNNEL_CLIENT_LIFECYCLE_EVENT_PATH',
    'TUNNEL_CLIENT_LIFECYCLE_SESSION_ID',
    'TUNNEL_CLIENT_LIFECYCLE_CONFIG_GENERATION',
    'TUNNEL_CLIENT_LIFECYCLE_SAFE_CONFIG_SHA256',
    'lattice.tunnel-client.lifecycle-event.v1',
    'lattice.tunnel-client.lifecycle-anomaly.v1',
    "'SPAWN', 'OPEN', 'CLOSE_REQUESTED', 'PIPE_CLOSED', 'EXITED', 'REAPED'",
    "lifecycle_classification -cne 'UNKNOWN'",
    'threshold_profile_version',
    'pipe_milliseconds',
    'exit_milliseconds',
    'reap_milliseconds',
    'confirm_milliseconds',
    'C_CALIBRATION_FIRST',
    'CreateJobObject',
    'CREATE_SUSPENDED',
    'AssignProcessToJobObject',
    'ResumeThread',
    '[Text.UTF8Encoding]::new($false, $true)',
    '$bytes[0] -eq 0xef',
    '$eventTypes[$eventIndex]',
    'TASK038_TUNNEL_LIFECYCLE_EVIDENCE_REJECTED'
)
foreach ($fragment in $lifecycleMaterializationFragments) {
    if ($tunnelLauncherText.IndexOf($fragment, [StringComparison]::Ordinal) -lt 0) {
        throw ('TASK038_TUNNEL_LIFECYCLE_NOT_MATERIALIZED|' + $fragment)
    }
}
if ($tunnelLauncherText.IndexOf('SKIP', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
    throw 'TASK038_TUNNEL_SKIP_PATH_REJECTED'
}

$postgresTunnelFragments = @(
    'RunTask038TunnelHook',
    'Enable-Task038TunnelStoreAuthority',
    "-Mode 'ManagedRun'",
    "ValidateScript({ `$_ -cmatch '\A[0-9a-f]{32}\z' })",
    "`$RunId -cnotmatch '\A[0-9a-f]{32}\z'",
    '@(5432, 64272, 55432)',
    'LATTICE_STORE_DAEMON_INSTANCE_ID',
    'LATTICE_STORE_AUTHORITY_REVISION',
    'LATTICE_STORE_AUTHORITY_HEAD_DIGEST'
    'identity_materialized'
    'restart_identity_verified'
    'system_identifier'
    'postgres_executable_native_identity'
    'psql_executable_native_identity'
    'pg_ctl_executable_native_identity'
    '882a5a073a88817f6c6d4c8827df1e4269ff226d52cf6f47c9883e91088c6345'
    'e43adb9c5032e7efc63eebb44c5d32b142b34e5f4207666fed2dc7a51d43b630'
    'abe89b0767a8cd0f956059aa5a5a93cd1042efc6194d000c2501da3e23babbd2'
    'HolderTtlSeconds'
    'lattice.task019.postgres-holder-authority.v1'
    'HOLDER_OPEN'
    'MARKER_CREATED'
    'INITIAL_POSTMASTER_READY'
    'INITIAL_POSTMASTER_STOPPED'
    'RESTART_POSTMASTER_READY'
    'CONSUMER_STARTED'
    'CONSUMER_EXITED'
    'HOLDER_STOPPED'
    'CLEANUP_COMPLETED'
    'LATTICE_TASK019_HOLDER_RECEIPT_PATH'
    'LATTICE_TASK019_HOLDER_NONCE'
    'LATTICE_TASK019_HOLDER_CONSUMER_SESSION_ID'
    'Get-Task019PostmasterRuntimeEvidence'
    '[IO.FileShare]::ReadWrite'
)
foreach ($fragment in $postgresTunnelFragments) {
    if ($postgresHarnessText.IndexOf($fragment, [StringComparison]::Ordinal) -lt 0) {
        throw ('TASK038_TUNNEL_POSTGRES_HOOK_NOT_MATERIALIZED|' + $fragment)
    }
}

$environmentHelper = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'task038-local-process-environment.ps1'))
. $environmentHelper
$nativeIdentityHelper = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'windows-native-path-identity.ps1'))
. $nativeIdentityHelper
$requiredFragments = @(
    'task038-local-process-environment.ps1',
    'OfficialCodexExecutable',
    'CodexAuthHome',
    'PostgresHost',
    'PostgresPort',
    'PostgresRunId',
    'PostgresDataDirectory',
    'windows-native-path-identity.ps1',
    "ValidateScript({ `$_ -cmatch '\A[0-9a-f]{32}\z' })",
    'Read-Task038StrictUtf8Text',
    'Get-Task019ProductionDatabaseName',
    'Assert-Task038PostgresNativeIdentity',
    '@(5432, 64272, 55432)',
    'LATTICE_TASK038_POSTGRES_PASSWORD',
    'LATTICE_WRITER_LEASE_MIGRATOR_URL',
    'LATTICE_WRITER_LEASE_RUNTIME_URL',
    'LATTICE_WRITER_LEASE_ADMIN_URL',
    "'--bin', 'latticed'",
    "'--target-dir', `$task038CargoTarget, '--target', `$cargoHostTarget",
    "`$cargoHostTarget = 'x86_64-pc-windows-msvc'",
    "@('CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET')",
    '$watchdogSeconds = $TimeoutSeconds + 15',
    'latticed.exe',
    'lattice_task_submit',
    'lattice_task_status',
    'CONTROLLED_CODEX_CANARY',
    'sameClientRequestId',
    'differentClientRequestId',
    'postgres_live',
    'SKIP:',
    'Restart-DisposablePostgres',
    'Invoke-BoundedNativeFileCapture',
    'New-FreshCodexExecutionHome',
    'Remove-FreshCodexExecutionHome',
    'Assert-PublicTaskStatusShape',
    'Assert-CompletedTaskStatus',
    'Assert-SamePublicTaskStatus',
    'Assert-Task038ServerMeta',
    'Assert-LegacyInitializeResponse',
    'Assert-StatelessDiscoverResponse',
    'Assert-ToolResultEnvelope',
    'Write-McpResponseSummary',
    'Read-Task038McpAcceptanceEvidence',
    'Read-Task038McpObservedEffectEvidence',
    'LATTICE_MCP_OBSERVED_EFFECT_PATH',
    'LATTICE_MCP_OBSERVED_EFFECT_NONCE',
    'PROCESS_PRIVATE_HMAC_OBSERVED_AT_EFFECT_BOUNDARY',
    'Get-Task038FailureClassification',
    'Stop-Task038Job',
    'Stop-Task038ProcessTree',
    'ActiveProcessCount',
    'TerminateJobObject',
    'CreateProcessW',
    'CREATE_SUSPENDED',
    'ResumeThread',
    'Start-Task038SuspendedProcess',
    'Resume-Task038SuspendedProcess',
    'create_suspended',
    'job_assigned_before_resume',
    'resumed_after_job_assignment',
    'job_active_processes_after_cleanup',
    "GetFolderPath([Environment+SpecialFolder]::Windows)",
    'TASK038_POWERSHELL_SIGNATURE_REJECTED',
    'credential_source_unchanged',
    'execution_home_removed',
    'plugins = false',
    '1a9bc2b325476a4679e5ad9202329c97952ed8ea958162bd0ffadd2196833189',
    'RedirectStandardOutput',
    'RedirectStandardError',
    'TASK038_NATIVE_PROCESS_TIMEOUT',
    'TASK038_NATIVE_OUTPUT_DISCARD_REJECTED',
    'postgres_restart_between_submit_and_status = $true',
    'LATTICE_FULL_CHAIN_RUN_MODE',
    'LATTICE_TASK_INGRESS_KIND',
    'LATTICE_TASK_INGRESS_PROFILE_SHA256',
    'FRESH',
    'RESUME_EXISTING',
    'server/discover',
    'codex_home_footprint',
    'Get-StableDirectoryFootprint',
    'TASK038_DIRECTORY_FOOTPRINT_NOT_STABLE',
    'TASK038_DIRECTORY_FOOTPRINT_SCAN_TIMEOUT',
    'TASK038_DIRECTORY_FOOTPRINT_ENTRY_LIMIT_REJECTED',
    'TASK038_DIRECTORY_FOOTPRINT_BYTE_LIMIT_REJECTED',
    'git_head',
    'ledger_fingerprint',
    'delivery_failure_stage',
    'delivery_failure_code_sha256',
    'TASK038_DATABASE_FAILURE_PROJECTION_REJECTED',
    'created_profile_adapter_commitment',
    'lattice.task-created-ingress-audit.v1',
    'git-after-status.json',
    'codex-home-after-status.json',
    'database-after-status.json',
    'TASK038_FRESH_STATUS_GIT_FOOTPRINT_REJECTED',
    'TASK038_FRESH_STATUS_CODEX_HOME_FOOTPRINT_REJECTED',
    'TASK038_FRESH_STATUS_DATABASE_FOOTPRINT_REJECTED',
    'LOCAL_CANONICAL_MCP_NOT_CHATGPT_TUNNEL',
    'chatgpt_tunnel_claimed = $false',
    'LATTICED_LOCAL_MCP_ACCEPTANCE=PASS'
    'LATTICE_MCP_ACCEPTANCE_EVIDENCE_PATH'
    'LATTICE_MCP_ACCEPTANCE_SESSION_ID'
    'LATTICE_MCP_ACCEPTANCE_SAFE_CONFIG_SHA256'
    'lattice.mcp.acceptance-dispatch.v1'
    'SESSION_OPEN'
    'DISPATCH_ACCEPTED'
    'SESSION_CLOSED'
    'Read-Task038McpAcceptanceEvidence'
    'lattice.task038.production-effect-observation.v1'
    'lattice.task038.candidate-source-linkage.v1'
    'exact_path_entries_sha256'
    'process_job_active_count_after_cleanup = 0'
    'network_tcp_owner_rows_after_cleanup'
    'network_udp_owner_rows_after_cleanup'
    'candidate_source_linkage_raw_sha256'
    'TASK038_MCP_ACCEPTANCE_EVIDENCE_REJECTED'
    'TASK038_CANDIDATE_SOURCE_REJECTED'
    'TASK038_POSTGRES_RUNTIME_BINDING_REJECTED'
)
foreach ($fragment in $requiredFragments) {
    if ($text.IndexOf($fragment, [StringComparison]::Ordinal) -lt 0) {
        throw ('TASK038_LOCAL_REQUIRED_FRAGMENT_MISSING|' + $fragment)
    }
}

$caseSensitiveRunIdValidator = "ValidateScript({ `$_ -cmatch '\A[0-9a-f]{32}\z' })"
if (
    $text.Split(
        [string[]]@($caseSensitiveRunIdValidator),
        [StringSplitOptions]::None
    ).Count - 1 -ne 11
) {
    throw 'TASK038_LOCAL_RUN_ID_VALIDATION_REJECTED'
}
foreach ($forbiddenRunIdValidation in @(
    "ValidatePattern('^[0-9a-f]{32}$')",
    "-match '^\[0-9a-f\]{32}$'"
)) {
    if ($text.IndexOf($forbiddenRunIdValidation, [StringComparison]::Ordinal) -ge 0) {
        throw 'TASK038_LOCAL_RUN_ID_CASE_INSENSITIVE_VALIDATION_REJECTED'
    }
}

$freshStatusEvidenceWrites = @(
    "Write-JsonEvidence -Path (Join-Path `$evidenceRoot 'git-after-status.json') -Value `$gitAfterStatus",
    "Write-JsonEvidence -Path (Join-Path `$evidenceRoot 'codex-home-after-status.json') -Value ([ordered]@{ codex_home_footprint = `$codexAfterStatus })",
    "Write-JsonEvidence -Path (Join-Path `$evidenceRoot 'database-after-status.json') -Value `$databaseAfterStatus"
)
$freshStatusFootprintGates = @(
    @{
        Compare = 'if (($gitAfterSubmit | ConvertTo-Json -Compress -Depth 8) -ne ($gitAfterStatus | ConvertTo-Json -Compress -Depth 8)) {'
        Failure = "throw 'TASK038_FRESH_STATUS_GIT_FOOTPRINT_REJECTED'"
    },
    @{
        Compare = 'if (($codexAfterSubmit | ConvertTo-Json -Compress -Depth 8) -ne ($codexAfterStatus | ConvertTo-Json -Compress -Depth 8)) {'
        Failure = "throw 'TASK038_FRESH_STATUS_CODEX_HOME_FOOTPRINT_REJECTED'"
    },
    @{
        Compare = 'if (($databaseAfterSubmit | ConvertTo-Json -Compress -Depth 8) -ne ($databaseAfterStatus | ConvertTo-Json -Compress -Depth 8)) {'
        Failure = "throw 'TASK038_FRESH_STATUS_DATABASE_FOOTPRINT_REJECTED'"
    }
)
$latestEvidenceWriteIndex = -1
foreach ($write in $freshStatusEvidenceWrites) {
    $writeIndex = $text.IndexOf($write, [StringComparison]::Ordinal)
    if ($writeIndex -lt 0 -or $writeIndex -le $latestEvidenceWriteIndex) {
        throw 'TASK038_FRESH_STATUS_EVIDENCE_ORDER_REJECTED'
    }
    $latestEvidenceWriteIndex = $writeIndex
}
foreach ($gate in $freshStatusFootprintGates) {
    $compareIndex = $text.IndexOf([string]$gate.Compare, [StringComparison]::Ordinal)
    $failureIndex = $text.IndexOf([string]$gate.Failure, [StringComparison]::Ordinal)
    if (
        $compareIndex -le $latestEvidenceWriteIndex -or
        $failureIndex -le $compareIndex
    ) {
        throw 'TASK038_FRESH_STATUS_GATE_ORDER_REJECTED'
    }
}
if ($text.IndexOf('TASK038_FRESH_STATUS_SIDE_EFFECT_REJECTED', [StringComparison]::Ordinal) -ge 0) {
    throw 'TASK038_FRESH_STATUS_COMBINED_GATE_REJECTED'
}

$submitEvidenceIndex = $text.IndexOf(
    "Write-JsonEvidence -Path (Join-Path `$evidenceRoot 'submit.json') -Value `$submitted",
    [StringComparison]::Ordinal
)
$submitDatabaseEvidenceIndex = $text.IndexOf(
    "Write-JsonEvidence -Path (Join-Path `$evidenceRoot 'database-after-submit.json') -Value `$databaseAfterSubmit",
    [StringComparison]::Ordinal
)
$submitCompletionGateIndex = $text.IndexOf(
    'Assert-CompletedTaskStatus -Value $submitted',
    [StringComparison]::Ordinal
)
if (
    $submitEvidenceIndex -lt 0 -or
    $submitDatabaseEvidenceIndex -le $submitEvidenceIndex -or
    $submitCompletionGateIndex -le $submitDatabaseEvidenceIndex
) {
    throw 'TASK038_FAILED_SUBMIT_EVIDENCE_ORDER_REJECTED'
}

$forbiddenPatterns = @(
    '(?i)run-task037',
    '(?i)start-chatgpt-mcp-tunnel',
    '(?i)lattice-full-chain',
    '(?i)openclaw\.mjs|LATTICE_OPENCLAW|OPENCLAW_PROFILE',
    '(?i)lattice-hermes|LATTICE_HERMES_',
    '(?i)offline-runtime-manifest',
    '(?i)full-chain-acceptance\\[0-9a-f]{24}',
    '(?im)\[string\]\s*\$PostgresPassword',
    '(?im)\[string\]\s*\$WriterLease(?:Migrator|Runtime|Admin)Url',
    '(?i)taskkill\.exe',
    "(?i)Get-Command\s+'powershell\.exe'"
)
foreach ($pattern in $forbiddenPatterns) {
    if ($text -match $pattern) {
        throw ('TASK038_LOCAL_FORBIDDEN_SURFACE|' + $pattern)
    }
}

$functionAsts = @($scriptAst.FindAll({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst]
}, $true))
function Get-HarnessFunctionAst {
    param([Parameter(Mandatory = $true)][string]$Name)

    $matches = @($functionAsts | Where-Object { $_.Name -ceq $Name })
    if ($matches.Count -ne 1) {
        throw ('TASK038_LOCAL_FUNCTION_SHAPE_REJECTED|' + $Name)
    }
    return $matches[0]
}

$latticedSessionText = (Get-HarnessFunctionAst -Name 'Invoke-LatticedSession').Extent.Text
$createSuspendedIndex = $latticedSessionText.IndexOf(
    '$suspendedProcess = Start-Task038SuspendedProcess -StartInfo $startInfo',
    [StringComparison]::Ordinal
)
$assignJobIndex = $latticedSessionText.IndexOf(
    'Add-Task038ProcessToJob -Job $jobHandle -Process $process',
    [StringComparison]::Ordinal
)
$resumeIndex = $latticedSessionText.IndexOf(
    'Resume-Task038SuspendedProcess -SuspendedProcess $suspendedProcess',
    [StringComparison]::Ordinal
)
$stopJobIndex = $latticedSessionText.IndexOf(
    'Stop-Task038Job -Job $jobHandle',
    [StringComparison]::Ordinal
)
$pipeWaitIndex = $latticedSessionText.IndexOf(
    '$stdoutTask.Wait(5000)',
    $stopJobIndex,
    [StringComparison]::Ordinal
)
if (
    $createSuspendedIndex -lt 0 -or
    $assignJobIndex -le $createSuspendedIndex -or
    $resumeIndex -le $assignJobIndex -or
    $stopJobIndex -le $resumeIndex -or
    $pipeWaitIndex -le $stopJobIndex
) {
    throw 'TASK038_LOCAL_SUSPENDED_JOB_PIPE_ORDER_REJECTED'
}

$restartText = (Get-HarnessFunctionAst -Name 'Restart-DisposablePostgres').Extent.Text
if ($restartText -match '(?i)Invoke-NativeText') {
    throw 'TASK038_RESTART_PIPE_CAPTURE_REJECTED'
}
if ([regex]::Matches($restartText, '(?i)Invoke-BoundedNativeFileCapture').Count -ne 3) {
    throw 'TASK038_RESTART_NATIVE_CALL_COUNT_REJECTED'
}

$nativeRunnerText = (Get-HarnessFunctionAst -Name 'Invoke-BoundedNativeFileCapture').Extent.Text
foreach ($fragment in @(
    'Start-Process',
    'RedirectStandardOutput',
    'RedirectStandardError',
    'WaitForExit($TimeoutSeconds * 1000)'
)) {
    if ($nativeRunnerText.IndexOf($fragment, [StringComparison]::Ordinal) -lt 0) {
        throw ('TASK038_NATIVE_RUNNER_FRAGMENT_REJECTED|' + $fragment)
    }
}
if ($nativeRunnerText -match '(?m)&\s*\$Executable') {
    throw 'TASK038_NATIVE_RUNNER_PIPE_CAPTURE_REJECTED'
}

$newExecutionHomeText = (Get-HarnessFunctionAst -Name 'New-FreshCodexExecutionHome').Extent.Text
foreach ($fragment in @(
    "Join-Path `$source 'auth.json'",
    '[IO.File]::Copy($sourceAuth, $authPath, $false)',
    '.lattice-task038-execution-owner-v1',
    'Test-PathOverlap',
    'Write-Task038ExclusiveBytes',
    'TASK038_CODEX_EXECUTION_HOME_NOT_FRESH'
)) {
    if ($newExecutionHomeText.IndexOf($fragment, [StringComparison]::Ordinal) -lt 0) {
        throw ('TASK038_CODEX_EXECUTION_HOME_FRAGMENT_REJECTED|' + $fragment)
    }
}
if ($newExecutionHomeText -match '(?i)Copy\([^\r\n]*(config|sessions|sqlite|plugins|skills|cache)') {
    throw 'TASK038_CODEX_EXECUTION_HOME_COPY_SCOPE_REJECTED'
}
$exclusiveWriterText = (Get-HarnessFunctionAst -Name 'Write-Task038ExclusiveBytes').Extent.Text
if ($exclusiveWriterText.IndexOf('[IO.FileMode]::CreateNew', [StringComparison]::Ordinal) -lt 0) {
    throw 'TASK038_CODEX_EXECUTION_HOME_EXCLUSIVE_WRITE_REJECTED'
}
$removeExecutionHomeText = (Get-HarnessFunctionAst -Name 'Remove-FreshCodexExecutionHome').Extent.Text
foreach ($fragment in @(
    'Test-ExactPath -Actual $executionHome -Expected $expected',
    "Join-Path `$executionHome '.lattice-task038-execution-owner-v1'",
    'Remove-Item -LiteralPath $executionHome -Recurse -Force'
)) {
    if ($removeExecutionHomeText.IndexOf($fragment, [StringComparison]::Ordinal) -lt 0) {
        throw ('TASK038_CODEX_EXECUTION_HOME_CLEANUP_FRAGMENT_REJECTED|' + $fragment)
    }
}
$stableFootprintText = (Get-HarnessFunctionAst -Name 'Get-StableDirectoryFootprint').Extent.Text
foreach ($fragment in @(
    '[Diagnostics.Stopwatch]::StartNew()',
    'Get-DirectoryFootprint',
    '-Root $Root',
    '-DeadlineStopwatch $stopwatch',
    '-DeadlineMilliseconds $deadlineMilliseconds',
    'TASK038_DIRECTORY_FOOTPRINT_NOT_STABLE'
)) {
    if ($stableFootprintText.IndexOf($fragment, [StringComparison]::Ordinal) -lt 0) {
        throw ('TASK038_STABLE_FOOTPRINT_FRAGMENT_REJECTED|' + $fragment)
    }
}
$directoryFootprintText = (Get-HarnessFunctionAst -Name 'Get-DirectoryFootprint').Extent.Text
foreach ($fragment in @(
    '[ValidateRange(1, 4096)][int]$MaxChildEntries = 4096',
    '[ValidateRange(1, 134217728)][long]$MaxTotalBytes = 134217728',
    '[ValidateRange(1, 67108864)][long]$MaxFileBytes = 67108864',
    'TASK038_DIRECTORY_FOOTPRINT_SCAN_TIMEOUT',
    'TASK038_DIRECTORY_FOOTPRINT_ENTRY_LIMIT_REJECTED',
    'TASK038_DIRECTORY_FOOTPRINT_BYTE_LIMIT_REJECTED'
)) {
    if ($directoryFootprintText.IndexOf($fragment, [StringComparison]::Ordinal) -lt 0) {
        throw ('TASK038_DIRECTORY_FOOTPRINT_BOUND_FRAGMENT_REJECTED|' + $fragment)
    }
}

foreach ($name in @(
    'Get-CanonicalPath',
    'Test-ExactPath',
    'Test-PathOverlap',
    'Assert-RegularFile',
    'Assert-NoReparseAncestor',
    'Assert-NoReparsePath',
    'Get-StringSha256',
    'Get-HmacStringSha256',
    'Get-FileSha256',
    'Read-Task038StrictUtf8Text',
    'Get-Task019ProductionDatabaseName',
    'Get-Task038FailureClassification',
    'Write-Task038ExclusiveBytes',
    'Assert-SecretFreeText',
    'Write-JsonEvidence',
    'Write-McpResponseSummary',
    'Set-Task038OwnerOnlyAcl',
    'New-Task038McpAcceptanceEvidenceSink',
    'Read-Task038McpAcceptanceEvidence',
    'New-Task038McpObservedEffectEvidenceSink',
    'Read-Task038McpObservedEffectEvidence',
    'Get-Task038CandidateSourceLinkage',
    'Get-DirectoryFootprint',
    'Get-StableDirectoryFootprint',
    'New-FreshCodexExecutionHome',
    'Remove-FreshCodexExecutionHome',
    'Assert-PublicTaskStatusShape',
    'Assert-CompletedTaskStatus',
    'Assert-SamePublicTaskStatus',
    'Assert-Task038ServerMeta',
    'Assert-LegacyInitializeResponse',
    'Assert-StatelessDiscoverResponse',
    'Assert-ToolResultEnvelope',
    'Get-ToolStructuredContent',
    'Initialize-Task038JobObjectInterop',
    'Initialize-Task038SuspendedProcessInterop',
    'New-Task038KillOnCloseJob',
    'Add-Task038ProcessToJob',
    'Start-Task038SuspendedProcess',
    'Resume-Task038SuspendedProcess',
    'Close-Task038Job',
    'Stop-Task038Job',
    'Stop-Task038ProcessTree',
    'Invoke-LatticedSession',
    'Invoke-BoundedNativeFileCapture'
)) {
    . ([scriptblock]::Create((Get-HarnessFunctionAst -Name $name).Extent.Text))
}

$script:RepositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$script:SecretValues = [Collections.Generic.List[string]]::new()
$probeRoot = [IO.Path]::GetFullPath((Join-Path $script:RepositoryRoot 'target\task038-native-runner-probe'))
if (-not (Test-Path -LiteralPath $probeRoot -PathType Container)) {
    New-Item -ItemType Directory -Path $probeRoot -Force:$false | Out-Null
}

try {
    $validRunId = '0123456789abcdef0123456789abcdef'
    if ((Get-Task019ProductionDatabaseName -RunId $validRunId) -cne 'lattice_task019_01234567_base') {
        throw 'TASK038_DATABASE_RUN_ID_VALIDATION_REJECTED'
    }
    foreach ($invalidRunId in @(
        '0123456789ABCDEF0123456789ABCDEF',
        '0123456789abcdef0123456789abcde',
        '0123456789abcdef0123456789abcdeg',
        ' 0123456789abcdef0123456789abcdef',
        '0123456789abcdef0123456789abcdef ',
        "0123456789abcdef0123456789abcdef`n"
    )) {
        $invalidRejected = $false
        try {
            $null = Get-Task019ProductionDatabaseName -RunId $invalidRunId
        }
        catch {
            $invalidRejected = $_.FullyQualifiedErrorId -like '*ParameterArgumentValidationError*'
        }
        if (-not $invalidRejected) {
            throw 'TASK038_DATABASE_RUN_ID_VALIDATION_REJECTED'
        }
    }

    $strictUtf8ValidPath = Join-Path $probeRoot 'strict-utf8-valid.json'
    $strictUtf8InvalidPath = Join-Path $probeRoot 'strict-utf8-invalid.json'
    $strictUtf8BomPath = Join-Path $probeRoot 'strict-utf8-bom.json'
    [IO.File]::WriteAllBytes(
        $strictUtf8ValidPath,
        [Text.UTF8Encoding]::new($false, $true).GetBytes('{"identity":"0123456789abcdef"}')
    )
    [IO.File]::WriteAllBytes($strictUtf8InvalidPath, [byte[]]@(0x7b, 0x22, 0x80, 0x22, 0x7d))
    [IO.File]::WriteAllBytes($strictUtf8BomPath, [byte[]]@(0xef, 0xbb, 0xbf, 0x7b, 0x7d))
    if ((Read-Task038StrictUtf8Text -Path $strictUtf8ValidPath -FailureCode 'STRICT_UTF8_REJECTED') -cne '{"identity":"0123456789abcdef"}') {
        throw 'TASK038_STRICT_UTF8_PROBE_REJECTED'
    }
    foreach ($invalidUtf8Path in @($strictUtf8InvalidPath, $strictUtf8BomPath)) {
        $strictUtf8Rejected = $false
        try {
            $null = Read-Task038StrictUtf8Text -Path $invalidUtf8Path -FailureCode 'STRICT_UTF8_REJECTED'
        }
        catch {
            $strictUtf8Rejected = [string]$_.Exception.Message -ceq 'STRICT_UTF8_REJECTED'
        }
        if (-not $strictUtf8Rejected) {
            throw 'TASK038_STRICT_UTF8_PROBE_REJECTED'
        }
    }
    foreach ($strictUtf8Path in @($strictUtf8ValidPath, $strictUtf8InvalidPath, $strictUtf8BomPath)) {
        [IO.File]::Delete($strictUtf8Path)
    }

    $nativeRoot = Join-Path $probeRoot 'native-containment'
    $nativeMarker = Join-Path $nativeRoot '.identity-marker'
    [IO.Directory]::CreateDirectory($nativeRoot) | Out-Null
    [IO.File]::WriteAllText($nativeMarker, "identity-v1`n", [Text.UTF8Encoding]::new($false))
    $nativeSnapshot = New-LatticeWindowsNativeContainmentSnapshot `
        -ParentPath $probeRoot `
        -RootPath $nativeRoot `
        -MarkerPath $nativeMarker
    Assert-LatticeWindowsNativeContainmentSnapshot `
        -Snapshot $nativeSnapshot `
        -FailureCode 'TASK038_NATIVE_IDENTITY_PROBE_REJECTED'
    [IO.File]::Delete($nativeMarker)
    [IO.File]::WriteAllText($nativeMarker, "identity-v1`n", [Text.UTF8Encoding]::new($false))
    if (Test-LatticeWindowsNativeContainmentSnapshot -Snapshot $nativeSnapshot) {
        throw 'TASK038_NATIVE_IDENTITY_REPLACEMENT_PROBE_REJECTED'
    }
    [IO.File]::Delete($nativeMarker)
    [IO.Directory]::Delete($nativeRoot, $false)

    $stableFootprintRoot = Join-Path $probeRoot 'stable-footprint'
    [IO.Directory]::CreateDirectory($stableFootprintRoot) | Out-Null
    try {
        $stableFootprintPath = Join-Path $stableFootprintRoot 'sentinel.txt'
        [IO.File]::WriteAllBytes($stableFootprintPath, [Text.Encoding]::UTF8.GetBytes('stable'))
        $stableFootprint = Get-StableDirectoryFootprint `
            -Root $stableFootprintRoot `
            -TimeoutSeconds 2 `
            -QuietMilliseconds 100
        if (
            $stableFootprint.file_count -ne 1 -or
            $stableFootprint.directory_count -ne 0 -or
            $stableFootprint.entry_count -ne 2 -or
            $stableFootprint.total_bytes -ne 6
        ) {
            throw 'TASK038_STABLE_FOOTPRINT_PROBE_REJECTED'
        }

        $beforeTimestampMutation = Get-DirectoryFootprint -Root $stableFootprintRoot
        $sentinelItem = Get-Item -LiteralPath $stableFootprintPath -Force
        $sentinelItem.LastWriteTimeUtc = $sentinelItem.LastWriteTimeUtc.AddSeconds(2)
        $afterTimestampMutation = Get-DirectoryFootprint -Root $stableFootprintRoot
        if ($beforeTimestampMutation.digest -eq $afterTimestampMutation.digest) {
            throw 'TASK038_DIRECTORY_FOOTPRINT_TIMESTAMP_PROBE_REJECTED'
        }

        [IO.File]::WriteAllBytes(
            (Join-Path $stableFootprintRoot 'second.txt'),
            [Text.Encoding]::UTF8.GetBytes('second')
        )
        $entryLimitRejected = $false
        try {
            $null = Get-DirectoryFootprint -Root $stableFootprintRoot -MaxChildEntries 1
        }
        catch {
            $entryLimitRejected = ([string]$_.Exception.Message -eq 'TASK038_DIRECTORY_FOOTPRINT_ENTRY_LIMIT_REJECTED')
        }
        if (-not $entryLimitRejected) {
            throw 'TASK038_DIRECTORY_FOOTPRINT_ENTRY_LIMIT_PROBE_REJECTED'
        }

        $byteLimitRejected = $false
        try {
            $null = Get-DirectoryFootprint -Root $stableFootprintRoot -MaxFileBytes 5
        }
        catch {
            $byteLimitRejected = ([string]$_.Exception.Message -eq 'TASK038_DIRECTORY_FOOTPRINT_BYTE_LIMIT_REJECTED')
        }
        if (-not $byteLimitRejected) {
            throw 'TASK038_DIRECTORY_FOOTPRINT_BYTE_LIMIT_PROBE_REJECTED'
        }

        $expiredStopwatch = [Diagnostics.Stopwatch]::StartNew()
        Start-Sleep -Milliseconds 10
        $scanTimeoutRejected = $false
        try {
            $null = Get-DirectoryFootprint `
                -Root $stableFootprintRoot `
                -DeadlineStopwatch $expiredStopwatch `
                -DeadlineMilliseconds 1
        }
        catch {
            $scanTimeoutRejected = ([string]$_.Exception.Message -eq 'TASK038_DIRECTORY_FOOTPRINT_SCAN_TIMEOUT')
        }
        if (-not $scanTimeoutRejected) {
            throw 'TASK038_DIRECTORY_FOOTPRINT_SCAN_TIMEOUT_PROBE_REJECTED'
        }

        $originalFootprintFunction = (Get-Item -LiteralPath Function:Get-DirectoryFootprint).ScriptBlock
        $script:continuousFootprintSequence = 0
        try {
            Set-Item -LiteralPath Function:Get-DirectoryFootprint -Value {
                param(
                    [string]$Root,
                    [Diagnostics.Stopwatch]$DeadlineStopwatch,
                    [long]$DeadlineMilliseconds
                )
                $script:continuousFootprintSequence += 1
                return [ordered]@{
                    file_count = 1
                    directory_count = 0
                    entry_count = 2
                    total_bytes = 6
                    digest = $script:continuousFootprintSequence.ToString('x64')
                }
            }
            $continuousChurnRejected = $false
            try {
                $null = Get-StableDirectoryFootprint `
                    -Root $stableFootprintRoot `
                    -TimeoutSeconds 1 `
                    -QuietMilliseconds 100
            }
            catch {
                $continuousChurnRejected = ([string]$_.Exception.Message -eq 'TASK038_DIRECTORY_FOOTPRINT_NOT_STABLE')
            }
            if (-not $continuousChurnRejected) {
                throw 'TASK038_DIRECTORY_FOOTPRINT_CONTINUOUS_CHURN_PROBE_REJECTED'
            }
        }
        finally {
            Set-Item -LiteralPath Function:Get-DirectoryFootprint -Value $originalFootprintFunction
            Remove-Variable -Name continuousFootprintSequence -Scope Script -ErrorAction SilentlyContinue
        }
    }
    finally {
        foreach ($leafName in @('sentinel.txt', 'second.txt')) {
            $leafPath = Join-Path $stableFootprintRoot $leafName
            if (Test-Path -LiteralPath $leafPath -PathType Leaf) {
                [IO.File]::Delete($leafPath)
            }
        }
        if (Test-Path -LiteralPath $stableFootprintRoot -PathType Container) {
            [IO.Directory]::Delete($stableFootprintRoot, $false)
        }
    }

    $probe = Invoke-BoundedNativeFileCapture `
        -Executable $env:ComSpec `
        -Arguments @('/d', '/c', 'echo', 'TASK038_NATIVE_FILE_CAPTURE_PROBE') `
        -OutputDirectory $probeRoot `
        -Operation 'PG_CTL_OUTPUT_PROBE' `
        -TimeoutSeconds 5
    if (
        $probe.ExitCode -ne 0 -or
        $probe.Stdout.Trim() -ne 'TASK038_NATIVE_FILE_CAPTURE_PROBE' -or
        -not [string]::IsNullOrEmpty($probe.Stderr)
    ) {
        throw 'TASK038_NATIVE_RUNNER_OUTPUT_REJECTED'
    }

    $nonzeroProbe = Invoke-BoundedNativeFileCapture `
        -Executable $env:ComSpec `
        -Arguments @('/d', '/c', 'exit', '/b', '3') `
        -OutputDirectory $probeRoot `
        -Operation 'PG_CTL_STATUS_EXIT_PROBE' `
        -TimeoutSeconds 5
    if (
        $nonzeroProbe.ExitCode -ne 3 -or
        -not [string]::IsNullOrEmpty($nonzeroProbe.Stdout) -or
        -not [string]::IsNullOrEmpty($nonzeroProbe.Stderr)
    ) {
        throw 'TASK038_NATIVE_RUNNER_NONZERO_EXIT_REJECTED'
    }

    $discardProbe = Invoke-BoundedNativeFileCapture `
        -Executable $env:ComSpec `
        -Arguments @('/d', '/c', 'echo TASK038_DISCARD_STDOUT ^& echo TASK038_DISCARD_STDERR 1^>^&2') `
        -OutputDirectory $probeRoot `
        -Operation 'PG_CTL_START' `
        -TimeoutSeconds 5 `
        -DiscardOutput
    if (
        $discardProbe.ExitCode -ne 0 -or
        -not [string]::IsNullOrEmpty($discardProbe.Stdout) -or
        -not [string]::IsNullOrEmpty($discardProbe.Stderr)
    ) {
        throw 'TASK038_NATIVE_RUNNER_DISCARD_OUTPUT_REJECTED'
    }

    $timeoutObserved = $false
    try {
        $null = Invoke-BoundedNativeFileCapture `
            -Executable ([string](Get-Command 'ping.exe' -CommandType Application -ErrorAction Stop).Source) `
            -Arguments @('-n', '6', '127.0.0.1') `
            -OutputDirectory $probeRoot `
            -Operation 'PG_CTL_TIMEOUT_PROBE' `
            -TimeoutSeconds 1
    }
    catch {
        $timeoutObserved = ([string]$_.Exception.Message -like 'TASK038_NATIVE_PROCESS_TIMEOUT|PG_CTL_TIMEOUT_PROBE|*')
    }
    if (-not $timeoutObserved) {
        throw 'TASK038_NATIVE_RUNNER_TIMEOUT_REJECTED'
    }
}
finally {
    if (Test-Path -LiteralPath $probeRoot -PathType Container) {
        $remaining = @(Get-ChildItem -LiteralPath $probeRoot -Force)
        if ($remaining.Count -ne 0) {
            throw 'TASK038_NATIVE_RUNNER_CLEANUP_REJECTED'
        }
        [IO.Directory]::Delete($probeRoot, $false)
    }
}

New-Item -ItemType Directory -Path $probeRoot -ErrorAction Stop | Out-Null
$processSentinel = Join-Path $probeRoot 'late-descendant-effect.txt'
$processPidPath = Join-Path $probeRoot 'descendant.pid'
$timeoutVariable = Get-Variable -Name TimeoutSeconds -ErrorAction SilentlyContinue
$originalTimeoutSeconds = if ($null -eq $timeoutVariable) { $null } else { $timeoutVariable.Value }
$script:Latticed = [string]$env:ComSpec
$script:PowerShell = [string](Get-Command 'powershell.exe' -CommandType Application -ErrorAction Stop).Source
$script:IngressProfileDigest = 'a' * 64
$script:SecretValues = [Collections.Generic.List[string]]::new()
$PostgresHost = '127.0.0.1'
$PostgresPort = 1
$PostgresRunId = 'e' * 32
$fakeAuthority = [pscustomobject]@{
    daemon_instance_id = 'task038-process-probe'
    daemon_epoch = 1
    authority_revision = 1
    observation_digest = ('b' * 64)
    head_digest = ('c' * 64)
}
try {
    $TimeoutSeconds = -10
    $childScript = (
        "[IO.File]::WriteAllText('$processPidPath',[string]`$PID);" +
        "Start-Sleep -Seconds 30;" +
        "[IO.File]::WriteAllText('$processSentinel','LATE')"
    )
    $childEncodedCommand = [Convert]::ToBase64String(
        [Text.Encoding]::Unicode.GetBytes($childScript)
    )
    $script:Latticed = $script:PowerShell
    $sessionInput = (
        "`$child = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile','-NonInteractive','-EncodedCommand','$childEncodedCommand') -PassThru;" +
        "Start-Sleep -Seconds 30;exit`r`n"
    )
    $timeoutRejected = $false
    $processProbeFailure = 'NONE'
    $timeoutEvidencePath = Join-Path $probeRoot 'timeout-dispatch.jsonl'
    [IO.File]::WriteAllBytes($timeoutEvidencePath, [byte[]]::new(0))
    $timeoutEvidenceIdentity = Get-LatticeWindowsNativePathIdentityToken `
        -Path $timeoutEvidencePath `
        -Directory $false
    try {
        $null = Invoke-LatticedSession `
            -InputText $sessionInput `
            -RunMode 'RESUME_EXISTING' `
            -OutputPath (Join-Path $probeRoot 'process-probe-response-summary.json') `
            -MetaPath (Join-Path $probeRoot 'process-probe-meta.json') `
            -Authority $fakeAuthority `
            -DatabasePassword 'TASK038_PROCESS_PROBE_PASSWORD' `
            -DeliveryRoot $probeRoot `
            -SchemaDirectory $probeRoot `
            -LauncherSha256 ('d' * 64) `
            -LauncherVersion 'task038-process-probe' `
            -AcceptanceEvidencePath $timeoutEvidencePath `
            -AcceptanceEvidenceNativeIdentity $timeoutEvidenceIdentity `
            -AcceptanceSessionId ('1' * 32) `
            -AcceptanceSafeConfigSha256 ('2' * 64) `
            -ExpectedDispatchCount 0
    }
    catch {
        $processProbeFailure = Get-Task038FailureClassification -ErrorRecord $_
        $timeoutRejected = ($processProbeFailure -eq 'TASK038_LATTICED_TIMEOUT')
    }
    if (-not $timeoutRejected) {
        throw ('TASK038_LATTICED_TIMEOUT_PROBE_REJECTED|' + $processProbeFailure)
    }
    Start-Sleep -Seconds 4
    if (Test-Path -LiteralPath $processSentinel) {
        throw 'TASK038_LATTICED_DESCENDANT_EFFECT_PROBE_REJECTED'
    }
    if (-not (Test-Path -LiteralPath $processPidPath -PathType Leaf)) {
        throw 'TASK038_LATTICED_DESCENDANT_START_PROBE_REJECTED'
    }
    $descendantProcessId = 0
    if (
        -not [int]::TryParse([IO.File]::ReadAllText($processPidPath).Trim(), [ref]$descendantProcessId) -or
        $null -ne (Get-Process -Id $descendantProcessId -ErrorAction SilentlyContinue)
    ) {
        throw 'TASK038_LATTICED_DESCENDANT_REAP_PROBE_REJECTED'
    }
    Remove-Item -LiteralPath $processPidPath -Force

    $earlyExitSentinel = Join-Path $probeRoot 'early-exit-descendant-effect.txt'
    $earlyExitPidPath = Join-Path $probeRoot 'early-exit-descendant.pid'
    $TimeoutSeconds = 60
    $earlyExitChildScript = (
        "[IO.File]::WriteAllText('$earlyExitPidPath',[string]`$PID);" +
        "Start-Sleep -Seconds 3;" +
        "[IO.File]::WriteAllText('$earlyExitSentinel','LATE')"
    )
    $earlyExitEncodedCommand = [Convert]::ToBase64String(
        [Text.Encoding]::Unicode.GetBytes($earlyExitChildScript)
    )
    $script:Latticed = [string](Get-Command 'powershell.exe' -CommandType Application -ErrorAction Stop).Source
    $earlyExitInput = (
        "`$child = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile','-NonInteractive','-EncodedCommand','$earlyExitEncodedCommand') -PassThru;" +
        "[IO.File]::WriteAllText('$earlyExitPidPath',[string]`$child.Id);" +
        "exit`r`n"
    )
    $earlyEvidencePath = Join-Path $probeRoot 'early-exit-dispatch.jsonl'
    [IO.File]::WriteAllBytes($earlyEvidencePath, [byte[]]::new(0))
    $earlyEvidenceIdentity = Get-LatticeWindowsNativePathIdentityToken `
        -Path $earlyEvidencePath `
        -Directory $false
    $earlyEvidenceRejected = $false
    $earlyEvidenceFailure = 'NONE'
    try {
        $null = Invoke-LatticedSession `
            -InputText $earlyExitInput `
            -RunMode 'RESUME_EXISTING' `
            -OutputPath (Join-Path $probeRoot 'early-exit-response-summary.json') `
            -MetaPath (Join-Path $probeRoot 'early-exit-meta.json') `
            -Authority $fakeAuthority `
            -DatabasePassword 'TASK038_PROCESS_PROBE_PASSWORD' `
            -DeliveryRoot $probeRoot `
            -SchemaDirectory $probeRoot `
            -LauncherSha256 ('d' * 64) `
            -LauncherVersion 'task038-process-probe' `
            -AcceptanceEvidencePath $earlyEvidencePath `
            -AcceptanceEvidenceNativeIdentity $earlyEvidenceIdentity `
            -AcceptanceSessionId ('3' * 32) `
            -AcceptanceSafeConfigSha256 ('4' * 64) `
            -ExpectedDispatchCount 0
    }
    catch {
        $earlyEvidenceFailure = Get-Task038FailureClassification -ErrorRecord $_
        $earlyEvidenceRejected = ($earlyEvidenceFailure -eq 'TASK038_MCP_ACCEPTANCE_EVIDENCE_REJECTED')
    }
    if (-not $earlyEvidenceRejected) {
        throw ('TASK038_MCP_ACCEPTANCE_EVIDENCE_FAIL_CLOSED_PROBE_REJECTED|' + $earlyEvidenceFailure)
    }
    Start-Sleep -Seconds 4
    if (
        (Test-Path -LiteralPath $earlyExitSentinel) -or
        -not (Test-Path -LiteralPath $earlyExitPidPath -PathType Leaf)
    ) {
        throw 'TASK038_LATTICED_EARLY_EXIT_DESCENDANT_EFFECT_PROBE_REJECTED'
    }
    $earlyExitProcessId = 0
    if (
        -not [int]::TryParse([IO.File]::ReadAllText($earlyExitPidPath).Trim(), [ref]$earlyExitProcessId) -or
        $null -ne (Get-Process -Id $earlyExitProcessId -ErrorAction SilentlyContinue)
    ) {
        throw 'TASK038_LATTICED_EARLY_EXIT_DESCENDANT_REAP_PROBE_REJECTED'
    }
    Remove-Item -LiteralPath $earlyExitPidPath -Force
}
finally {
    if ($null -eq $timeoutVariable) {
        Remove-Variable -Name TimeoutSeconds -ErrorAction SilentlyContinue
    }
    else {
        $TimeoutSeconds = $originalTimeoutSeconds
    }
    foreach ($path in @(
        $processSentinel,
        $processPidPath,
        (Join-Path $probeRoot 'early-exit-descendant-effect.txt'),
        (Join-Path $probeRoot 'early-exit-descendant.pid'),
        (Join-Path $probeRoot 'process-probe-response-summary.json'),
        (Join-Path $probeRoot 'process-probe-meta.json'),
        (Join-Path $probeRoot 'early-exit-response-summary.json'),
        (Join-Path $probeRoot 'early-exit-meta.json'),
        (Join-Path $probeRoot 'timeout-dispatch.jsonl'),
        (Join-Path $probeRoot 'early-exit-dispatch.jsonl'),
        (Join-Path $probeRoot 'mcp-observed-effects\11111111111111111111111111111111.jsonl'),
        (Join-Path $probeRoot 'mcp-observed-effects\33333333333333333333333333333333.jsonl')
    )) {
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            Remove-Item -LiteralPath $path -Force
        }
    }
    $observedEffectProbeRoot = Join-Path $probeRoot 'mcp-observed-effects'
    if (Test-Path -LiteralPath $observedEffectProbeRoot -PathType Container) {
        [IO.Directory]::Delete($observedEffectProbeRoot, $false)
    }
    if (Test-Path -LiteralPath $probeRoot -PathType Container) {
        $remaining = @(Get-ChildItem -LiteralPath $probeRoot -Force)
        if ($remaining.Count -ne 0) {
            throw 'TASK038_LATTICED_PROCESS_PROBE_CLEANUP_REJECTED'
        }
        [IO.Directory]::Delete($probeRoot, $false)
    }
}

$digest = 'a' * 64
$completedStatus = [pscustomobject][ordered]@{
    ledger_head_digest = $digest
    result_digest = $digest
    schema_version = 'lattice.task.status.v1'
    status = 'COMPLETED'
    task_ref = $digest
    task_state = 'COMPLETED'
}
Assert-PublicTaskStatusShape -Value $completedStatus
Assert-CompletedTaskStatus -Value $completedStatus

$failedStatus = [pscustomobject][ordered]@{
    ledger_head_digest = $digest
    result_digest = $null
    schema_version = 'lattice.task.status.v1'
    status = 'FAILED'
    task_ref = $digest
    task_state = 'FAILED'
}
Assert-PublicTaskStatusShape -Value $failedStatus
$completionRejected = $false
try {
    Assert-CompletedTaskStatus -Value $failedStatus
}
catch {
    $completionRejected = ([string]$_.Exception.Message -eq 'TASK038_TASK_NOT_COMPLETED')
}
if (-not $completionRejected) {
    throw 'TASK038_PUBLIC_STATUS_COMPLETION_GATE_REJECTED'
}

$reconciliationStatus = [pscustomobject][ordered]@{
    ledger_head_digest = $digest
    result_digest = $null
    schema_version = 'lattice.task.status.v1'
    status = 'RECONCILIATION_REQUIRED'
    task_ref = $digest
    task_state = 'EXECUTING'
}
Assert-PublicTaskStatusShape -Value $reconciliationStatus
$extraStatus = $completedStatus.PSObject.Copy()
$extraStatus | Add-Member -NotePropertyName fencing_token -NotePropertyValue 1
$extraRejected = $false
try {
    Assert-PublicTaskStatusShape -Value $extraStatus
}
catch {
    $extraRejected = ([string]$_.Exception.Message -eq 'TASK038_PUBLIC_STATUS_SHAPE_REJECTED')
}
if (-not $extraRejected) {
    throw 'TASK038_PUBLIC_STATUS_EXTRA_FIELD_REJECTED'
}

$statusText = $completedStatus | ConvertTo-Json -Compress -Depth 8
$closedResponse = [pscustomobject][ordered]@{
    jsonrpc = '2.0'
    id = 3
    result = [pscustomobject][ordered]@{
        content = @([pscustomobject][ordered]@{ type = 'text'; text = $statusText })
        isError = $false
        structuredContent = $completedStatus
    }
}
$null = Get-ToolStructuredContent -Response $closedResponse -ExpectedKind 'TASK_STATUS'
$hostileResponse = $closedResponse.PSObject.Copy()
$hostileResult = $closedResponse.result.PSObject.Copy()
$hostileResult | Add-Member -NotePropertyName _meta -NotePropertyValue ([pscustomobject]@{ token = 'TASK038_FAKE_TOKEN_SENTINEL' })
$hostileResponse.result = $hostileResult
$envelopeRejected = $false
try {
    $null = Get-ToolStructuredContent -Response $hostileResponse -ExpectedKind 'TASK_STATUS'
}
catch {
    $envelopeRejected = ([string]$_.Exception.Message -eq 'TASK038_TOOL_RESULT_ENVELOPE_REJECTED')
}
if (-not $envelopeRejected) {
    throw 'TASK038_TOOL_RESULT_EXTRA_FIELD_REJECTED'
}

$serverInstructions = 'Four bounded LATTICE tools. Authority, task binding, orchestration, and execution configuration remain server-owned.'
$serverInfo = [pscustomobject][ordered]@{
    name = 'latticed'
    title = 'LATTICE DevOS'
    version = '1.0.0'
}
$initializeResponse = [pscustomobject][ordered]@{
    jsonrpc = '2.0'
    id = 1
    result = [pscustomobject][ordered]@{
        protocolVersion = '2025-11-25'
        capabilities = [pscustomobject][ordered]@{ tools = [pscustomobject]@{} }
        serverInfo = $serverInfo
        instructions = $serverInstructions
    }
}
Assert-LegacyInitializeResponse -Response $initializeResponse
$hostileInitialize = $initializeResponse.PSObject.Copy()
$hostileInitializeResult = $initializeResponse.result.PSObject.Copy()
$hostileInitializeResult | Add-Member -NotePropertyName token -NotePropertyValue 'TASK038_FAKE_TOKEN_SENTINEL'
$hostileInitialize.result = $hostileInitializeResult
$initializeRejected = $false
try {
    Assert-LegacyInitializeResponse -Response $hostileInitialize
}
catch {
    $initializeRejected = ([string]$_.Exception.Message -eq 'TASK038_MCP_INITIALIZE_RESPONSE_REJECTED')
}
if (-not $initializeRejected) {
    throw 'TASK038_MCP_INITIALIZE_HOSTILE_ENVELOPE_REJECTED'
}

$serverMeta = [pscustomobject][ordered]@{
    'io.modelcontextprotocol/serverInfo' = $serverInfo
}
$discoverResponse = [pscustomobject][ordered]@{
    jsonrpc = '2.0'
    id = 1
    result = [pscustomobject][ordered]@{
        resultType = 'complete'
        supportedVersions = @('2026-07-28')
        capabilities = [pscustomobject][ordered]@{ tools = [pscustomobject]@{} }
        instructions = $serverInstructions
        ttlMs = 0
        cacheScope = 'private'
        _meta = $serverMeta
    }
}
Assert-StatelessDiscoverResponse -Response $discoverResponse
$hostileDiscover = $discoverResponse.PSObject.Copy()
$hostileDiscoverResult = $discoverResponse.result.PSObject.Copy()
$hostileMeta = $serverMeta.PSObject.Copy()
$hostileMeta | Add-Member -NotePropertyName token -NotePropertyValue 'TASK038_FAKE_TOKEN_SENTINEL'
$hostileDiscoverResult._meta = $hostileMeta
$hostileDiscover.result = $hostileDiscoverResult
$discoverRejected = $false
try {
    Assert-StatelessDiscoverResponse -Response $hostileDiscover
}
catch {
    $discoverRejected = ([string]$_.Exception.Message -eq 'TASK038_MCP_DISCOVER_RESPONSE_REJECTED')
}
if (-not $discoverRejected) {
    throw 'TASK038_MCP_DISCOVER_HOSTILE_ENVELOPE_REJECTED'
}

$summaryPath = Join-Path $probeRoot 'response-summary.json'
Write-McpResponseSummary -Path $summaryPath -ResponseText 'TASK038_FAKE_TOKEN_SENTINEL'
$summaryText = [IO.File]::ReadAllText($summaryPath)
if ($summaryText.Contains('TASK038_FAKE_TOKEN_SENTINEL')) {
    throw 'TASK038_RAW_MCP_EVIDENCE_REJECTED'
}
Remove-Item -LiteralPath $summaryPath -Force
if (
    (Test-Path -LiteralPath $probeRoot -PathType Container) -and
    @(Get-ChildItem -LiteralPath $probeRoot -Force).Count -eq 0
) {
    [IO.Directory]::Delete($probeRoot, $false)
}

$homeProbeRoot = Get-CanonicalPath -Path (Join-Path ([IO.Path]::GetTempPath()) ('lattice-task038-home-probe-' + [Guid]::NewGuid().ToString('N')))
$homeProbeMarker = Join-Path $homeProbeRoot '.lattice-task038-home-probe-v1'
$originalCodexHome = $env:CODEX_HOME
try {
    New-Item -ItemType Directory -Path $homeProbeRoot -ErrorAction Stop | Out-Null
    [IO.File]::WriteAllText($homeProbeMarker, "lattice.task038.home-probe.v1`n", [Text.UTF8Encoding]::new($false))
    $credentialSource = Join-Path $homeProbeRoot 'credential-source'
    New-Item -ItemType Directory -Path $credentialSource -ErrorAction Stop | Out-Null
    [IO.File]::WriteAllText((Join-Path $credentialSource '.lattice-codex-home-v1'), "lattice.codex-home.v1`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $credentialSource 'auth.json'), '{"opaque":"TASK038_FAKE_AUTH_SENTINEL"}', [Text.UTF8Encoding]::new($false))
    New-Item -ItemType Directory -Path (Join-Path $credentialSource 'sessions') -ErrorAction Stop | Out-Null
    [IO.File]::WriteAllText((Join-Path $credentialSource 'config.toml'), 'source-config-must-not-copy', [Text.UTF8Encoding]::new($false))
    $sourceBefore = Get-DirectoryFootprint -Root $credentialSource
    $acceptanceId = [Guid]::NewGuid().ToString('N')
    $executionEvidence = New-FreshCodexExecutionHome -CredentialSource $credentialSource -RepositoryRoot $script:RepositoryRoot -AcceptanceId $acceptanceId
    $executionItems = @(Get-ChildItem -LiteralPath $executionEvidence.Path -Force)
    if (
        $executionItems.Count -ne 4 -or
        (Get-FileSha256 -Path (Join-Path $executionEvidence.Path 'config.toml')) -ne '1a9bc2b325476a4679e5ad9202329c97952ed8ea958162bd0ffadd2196833189' -or
        (Test-Path -LiteralPath (Join-Path $executionEvidence.Path 'sessions'))
    ) {
        throw 'TASK038_CODEX_EXECUTION_HOME_PROBE_REJECTED'
    }

    $reuseRejected = $false
    try {
        $null = New-FreshCodexExecutionHome -CredentialSource $credentialSource -RepositoryRoot $script:RepositoryRoot -AcceptanceId $acceptanceId
    }
    catch {
        $reuseRejected = ([string]$_.Exception.Message -eq 'TASK038_CODEX_EXECUTION_HOME_NOT_FRESH')
    }
    if (-not $reuseRejected) { throw 'TASK038_CODEX_EXECUTION_HOME_REUSE_PROBE_REJECTED' }

    $collisionId = [Guid]::NewGuid().ToString('N')
    $collisionPath = Join-Path $executionEvidence.Parent $collisionId
    New-Item -ItemType Directory -Path $collisionPath -ErrorAction Stop | Out-Null
    [IO.File]::WriteAllText((Join-Path $collisionPath 'foreign.txt'), 'foreign', [Text.UTF8Encoding]::new($false))
    $collisionRejected = $false
    try {
        $null = New-FreshCodexExecutionHome -CredentialSource $credentialSource -RepositoryRoot $script:RepositoryRoot -AcceptanceId $collisionId
    }
    catch {
        $collisionRejected = ([string]$_.Exception.Message -eq 'TASK038_CODEX_EXECUTION_HOME_NOT_FRESH')
    }
    if (-not $collisionRejected -or (Test-Path -LiteralPath (Join-Path $collisionPath 'auth.json'))) {
        throw 'TASK038_CODEX_EXECUTION_HOME_COLLISION_PROBE_REJECTED'
    }
    Remove-Item -LiteralPath $collisionPath -Recurse -Force

    $lockedId = [Guid]::NewGuid().ToString('N')
    $authLock = [IO.File]::Open((Join-Path $credentialSource 'auth.json'), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    try {
        $partialRejected = $false
        try {
            $null = New-FreshCodexExecutionHome -CredentialSource $credentialSource -RepositoryRoot $script:RepositoryRoot -AcceptanceId $lockedId
        }
        catch {
            $partialRejected = ([string]$_.Exception.Message -in @(
                'TASK038_CODEX_EXECUTION_HOME_PROVISION_REJECTED',
                'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
            ))
        }
        if (-not $partialRejected -or (Test-Path -LiteralPath (Join-Path $executionEvidence.Parent $lockedId))) {
            throw 'TASK038_CODEX_EXECUTION_HOME_PARTIAL_PROBE_REJECTED'
        }
    }
    finally {
        $authLock.Dispose()
    }

    $overlapRejected = $false
    try {
        $null = New-FreshCodexExecutionHome -CredentialSource $credentialSource -RepositoryRoot $credentialSource -AcceptanceId ([Guid]::NewGuid().ToString('N'))
    }
    catch {
        $overlapRejected = ([string]$_.Exception.Message -eq 'TASK038_CODEX_CREDENTIAL_SOURCE_REPOSITORY_OVERLAP')
    }
    if (-not $overlapRejected) { throw 'TASK038_CODEX_EXECUTION_HOME_OVERLAP_PROBE_REJECTED' }

    $env:CODEX_HOME = $homeProbeRoot
    $ambientRejected = $false
    try {
        $null = New-FreshCodexExecutionHome -CredentialSource $credentialSource -RepositoryRoot $script:RepositoryRoot -AcceptanceId ([Guid]::NewGuid().ToString('N'))
    }
    catch {
        $ambientRejected = ([string]$_.Exception.Message -eq 'TASK038_AMBIENT_CODEX_HOME_REJECTED')
    }
    if (-not $ambientRejected) { throw 'TASK038_CODEX_EXECUTION_HOME_AMBIENT_PROBE_REJECTED' }
    $env:CODEX_HOME = $originalCodexHome

    $cleanupLock = [IO.File]::Open((Join-Path $executionEvidence.Path 'cleanup.lock'), [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $cleanupRejected = $false
        try {
            Remove-FreshCodexExecutionHome -Path $executionEvidence.Path -ExpectedParent $executionEvidence.Parent -AcceptanceId $acceptanceId
        }
        catch {
            $cleanupRejected = ([string]$_.Exception.Message -eq 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED')
        }
        if (-not $cleanupRejected -or (Test-Path -LiteralPath (Join-Path $executionEvidence.Path 'auth.json'))) {
            throw 'TASK038_CODEX_EXECUTION_HOME_SECRET_CLEANUP_PROBE_REJECTED'
        }
    }
    finally {
        $cleanupLock.Dispose()
    }
    Remove-FreshCodexExecutionHome -Path $executionEvidence.Path -ExpectedParent $executionEvidence.Parent -AcceptanceId $acceptanceId
    $sourceAfter = Get-DirectoryFootprint -Root $credentialSource
    if (($sourceBefore | ConvertTo-Json -Compress -Depth 8) -ne ($sourceAfter | ConvertTo-Json -Compress -Depth 8)) {
        throw 'TASK038_CODEX_CREDENTIAL_SOURCE_PROBE_MUTATED'
    }
}
finally {
    $env:CODEX_HOME = $originalCodexHome
    if (Test-Path -LiteralPath $homeProbeRoot -PathType Container) {
        $canonicalProbe = Get-CanonicalPath -Path $homeProbeRoot
        $canonicalTemp = Get-CanonicalPath -Path ([IO.Path]::GetTempPath())
        if (
            -not $canonicalProbe.StartsWith(($canonicalTemp + [IO.Path]::DirectorySeparatorChar), [StringComparison]::OrdinalIgnoreCase) -or
            -not (Test-Path -LiteralPath $homeProbeMarker -PathType Leaf)
        ) {
            throw 'TASK038_CODEX_EXECUTION_HOME_PROBE_CLEANUP_REJECTED'
        }
        Remove-Item -LiteralPath $canonicalProbe -Recurse -Force
    }
}

Write-Output 'TASK038_LOCAL_ACCEPTANCE_STATIC=PASS'
