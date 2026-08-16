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
    'TASK050_FULLY_VERIFIED',
    '8e5ba40d38b781afff7028841bd981c8dd2b9721',
    'mcpServerStatus/list',
    'app-server',
    '--stdio',
    '$script:Task051ExpectedCurrentCodexRelativePath = ''OpenAI\Codex\bin\e305f1c75d8da435\codex.exe''',
    '$script:Task051ExpectedCurrentCodexVersion = ''codex-cli 0.148.0-alpha.9''',
    '$script:Task051ExpectedCurrentCodexSha256 = ''f29f609375f3731d8db507a95124862a84e306982e30ba4300ddce5638bc6946''',
    'Get-Task051CurrentCodexFileIdentity',
    'codex_native_identity',
    'codex_creation_file_time_utc',
    'CodexAuthority',
    'lattice-task051-acceptance/',
    'unknown (lattice-task051-acceptance; 1)',
    '--ephemeral',
    '--json',
    'lattice_delivery_run',
    'lattice_delivery_status',
    'lattice_task_submit',
    'lattice_task_status',
    'Invoke-Task051CodexTool',
    'lattice.task.status.v1',
    'New-Task051OwnerOnlyDirectory',
    'New-Task051AtomicOwnerOnlyEmptyDirectory',
    'Assert-Task051AtomicDirectoryStage',
    'Get-Task051RunSlot',
    'Get-Task051RunRootMarkerText',
    'New-Task051RunRoot',
    'Assert-Task051RunRoot',
    'Assert-Task051RunRootMarker',
    'lattice.task051.run-root.v1',
    'TASK051_RUN_ROOT_MARKER_REJECTED',
    'TASK051_RUN_ROOT_SLOT_EXHAUSTED',
    'TASK051_ATOMIC_DIRECTORY_SELF_TEST=PASS',
    'TASK051_COMPACT_RUN_ROOT_SELF_TEST=PASS',
    'Start-Task038SuspendedProcess',
    'Add-Task038ProcessToJob',
    'Stop-Task038Job',
    'Close-Task038Job',
    'Stop-Task038ProcessTree',
    'TASK051_PROCESS_START_CLEANUP_REJECTED',
    'Assert-Task051OfficialCodexBundle',
    'Copy-Task051OfficialCodexBundle',
    'codex-official\0.146.0\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe',
    'codex-windows-sandbox-setup.exe',
    'codex-command-runner.exe',
    'codex-code-mode-host.exe',
    'codex-path\rg.exe',
    'codex-package.json',
    '@openai\codex\package.json',
    'bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb',
    'c12d225b34e7f82cdab6bbc714797abed661f40e158104694953889750121cef',
    '0102fa1820ecd03bb03a991fd2303a1a484118f7da8a71864f88ec94bca61d6d',
    '6ef1de0e04d859f8f4f6d4d64f0f3ceeec28658423d91de160f5e804280d1c36',
    '14231169855ec5205cf5a1b6f1db358ff4aed4247c86b69ce8aae647c77f6680',
    'aaa0646d6b615da94187b51efd50c69621a00867761161ae55cc16cfd545bec7',
    '24dd8c63a4d2b7bc2ded86c887974f842093ce4f2ed8473267a91e036c38da20',
    'TASK051_OFFICIAL_CODEX_BUNDLE_REJECTED',
    'TASK051_OFFICIAL_CODEX_BUNDLE_COPY_REJECTED',
    'TASK051_OFFICIAL_CODEX_BUNDLE_CLEANUP_REJECTED',
    'Initialize-Task051CargoHome',
    'CARGO_HOME',
    'CARGO_NET_OFFLINE',
    'TASK051_CARGO_HOME_CLEANUP_REJECTED',
    'CARGO_TARGET_DIR',
    'TASK051_TASK038_CARGO_HOST_REJECTED',
    'TASK051_TASK038_DELIVERY_ROOT_TRANSFORM_REJECTED',
    'TASK051_TASK038_DELIVERY_REPOSITORY_TRANSFORM_REJECTED',
    'TASK038_DELIVERY_ROOT_NOT_FRESH_REJECTED',
    'TASK038_DELIVERY_ROOT_CREATE_REJECTED',
    'TASK038_DELIVERY_ROOT_ACL_REJECTED',
    'TASK038_DELIVERY_TASK_REF_REJECTED',
    'TASK038_DELIVERY_TASK_ROOT_REJECTED',
    'TASK038_DELIVERY_REPOSITORY_REJECTED',
    'TASK051_CARGO_LINK_PATH_BUDGET_REJECTED',
    'TASK051_TASK038_HOLDER_EVENT_SEQUENCE_TRANSFORM_REJECTED',
    'TASK076_WRITER_V2_VERIFIED',
    'Get-Task051UniqueMcpServer',
    'Get-Task051McpToolNames',
    'TASK038_CURRENT_CODEX_DISCOVERY_SERVER_COUNT_ZERO_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_LATTICE_SERVER_COUNT_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_LATTICE_SERVER_ZERO_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_LATTICE_SERVER_DUPLICATE_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_TOOL_SHAPE_REJECTED',
    'TASK051_APP_SERVER_LATTICE_SERVER_SELECTION_SELF_TEST_REJECTED',
    'TASK051_APP_SERVER_TOOL_MAP_SELF_TEST_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_TOOL_COUNT_ZERO_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_TOOL_COUNT_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_TOOL_NAMES_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_RESOURCES_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_RESOURCE_TEMPLATES_REJECTED',
    'Read-Task051McpSessionOpen',
    'Test-Task051McpSessionOpenReady',
    'Get-Task051OwnedProcessEvidence',
    'LatticeTask051OwnedProcessAuthority',
    '::Acquire(',
    '.IsAlive()',
    '.CloseExact()',
    '[scriptblock]$PollAction',
    'IsProcessInJob',
    'QueryFullProcessImageName',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_TIMEOUT_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_READY_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_SOURCE_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_READ_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_FRAMING_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_JSON_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_KEYS_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_FIELDS_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_HASH_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_PROJECTION_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_POLL_RESULT_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_REPLAY_READ_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_REPLAY_MISMATCH_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_JOB_MEMBERSHIP_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_INTEROP_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_INTEROP_INIT_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_OPEN_ACCESS_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_OPEN_STALE_PID_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_OPEN_OTHER_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_JOB_QUERY_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_IMAGE_QUERY_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_TIME_QUERY_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_CLOSE_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_FILE_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_NATIVE_IDENTITY_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_SHA256_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_CREATION_REJECTED',
    'TASK051_RETAINED_PROCESS_AUTHORITY_SELF_TEST_CLEANUP_REJECTED',
    'TASK051_TASK038_CANDIDATE_BINARY_COMMITMENT_TRANSFORM_REJECTED',
    '$candidateLatticedNativeIdentity = Get-LatticeWindowsNativePathIdentityToken',
    '-ExpectedLatticedSha256 $candidateLatticedSha256',
    '-ExpectedLatticedNativeIdentity $candidateLatticedNativeIdentity',
    'TASK051_MCP_SESSION_OPEN_SELF_TEST_REJECTED',
    'TEMP',
    'TMP',
    'TASK051_TEMP_CLEANUP_REJECTED',
    'New-Task051RunRootAlias',
    'Remove-Task051RunRootAlias',
    'TASK051_RUN_ALIAS_CLEANUP_REJECTED',
    'Get-Task051PostgresProcessSnapshot',
    'Test-Task051PostgresProcessSnapshotClosed',
    'TASK051_POSTGRES_PROCESS_SNAPSHOT_SELF_TEST_REJECTED',
    'Test-Task051RunRootAliasReleaseSafe',
    'TASK051_RUN_ALIAS_PRESERVED_FOR_ACTIVE_RESOURCE',
    'LATTICE_TASK051_RUN_ALIAS_ROOT',
    'TASK051_TASK038_POSTGRES_DATA_ALIAS_TRANSFORM_REJECTED',
    'TASK051_TASK038_POSTGRES_DATA_ALIAS_SELF_TEST_REJECTED',
    'TASK038_POSTGRES_DATA_NATIVE_LINK_REJECTED',
    'TASK051_TASK038_EXECUTION_HOME_SHORT_PATH_TRANSFORM_REJECTED',
    'TASK051_TASK038_EXECUTION_HOME_BOUNDARY_TRANSFORM_REJECTED',
    'TASK051_TASK038_LONG_PATH_FOOTPRINT_ROOT_TRANSFORM_REJECTED',
    'TASK051_TASK038_LONG_PATH_CLEANUP_ROOT_TRANSFORM_REJECTED',
    'TASK051_TASK038_LONG_PATH_CLEANUP_DELETE_TRANSFORM_REJECTED',
    'TASK051_TASK038_LONG_PATH_CLEANUP_OWNER_ORDER_TRANSFORM_REJECTED',
    'TASK051_LONG_PATH_IO_SELF_TEST_REJECTED',
    'TASK051_SELF_TEST_ROOT_CLEANUP_REJECTED',
    'TASK051_TASK019_PGDATA_ALIAS_TRANSFORM_REJECTED',
    'TASK051_TASK019_RUNTIME_PGDATA_ALIAS_SELF_TEST_REJECTED',
    'TASK051_TASK019_CLUSTER_CLEANUP_TRANSFORM_REJECTED',
    'TASK051_TASK019_CLUSTER_CLEANUP_REJECTED',
    "PSObject.Properties['outputSchema']",
    'lattice.task051.current-codex-discovery.v2',
    'function Test-Task051ProcessLifetime',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_LIFETIME_REJECTED',
    'latticed_exit_file_time_utc',
    'latticed_was_running_at_capture',
    'ProcessQueryLimitedInformation | Synchronize',
    'WaitForSingleObject(authority.ProcessHandle, 0)',
    'WasRunningAtCapture',
    'authority.WasRunningAtCapture = waitResult == WaitTimeout;',
    'authority.ExitFileTimeUtc = 0;',
    'authority.ExitFileTimeUtc = checked((Int64)exitValue);',
    'if (waitResult != WaitTimeout && waitResult != WaitObject0)',
    'lattice.task051.current-codex-tool-call.v2',
    'lattice.task051.p0-platform-live-acceptance.v2',
    'lattice.task051.codex-result-meta-commitment.v1',
    'function Get-Task051CodexResultMetaCommitment',
    'TASK051_CODEX_TOOL_RESULT_META_OPTIONAL_SELF_TEST_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_META_COMMITMENT_SELF_TEST_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_META_EVIDENCE_SELF_TEST=PASS',
    'lattice.task051.codex-event-summary.v1',
    'function Get-Task051CodexEventSummary',
    'function Assert-Task051CodexPhaseTool',
    'TASK038_CURRENT_CODEX_TOOL_SUBMIT_CALL_COUNT_ZERO_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_SUBMIT_CALL_COUNT_MULTIPLE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_PRE_RESTART_CALL_COUNT_ZERO_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_PRE_RESTART_CALL_COUNT_MULTIPLE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_POST_RESTART_CALL_COUNT_ZERO_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_POST_RESTART_CALL_COUNT_MULTIPLE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_EVENT_SUMMARY_REJECTED',
    'collab_tool_call',
    'TASK051_CODEX_UNEXPECTED_TOOL_REJECTED',
    'function Resolve-Task051CodexToolFailure',
    'TASK038_CURRENT_CODEX_TOOL_HOME_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_START_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_PROCESS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_PROMPT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_WAIT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_EXIT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_OUTPUT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_EVENT_JSON_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_UNEXPECTED_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_SERVER_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_NAME_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_PUBLIC_KIND_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_ARGUMENT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_ERROR_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_MISSING_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_KEYS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_CONTENT_SHAPE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_META_SHAPE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_META_IDENTITY_REJECTED',
    'function Resolve-Task051PublicProjectionFailure',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_STRUCTURED_NULL_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_STRUCTURED_PUBLIC_SHAPE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_STRUCTURED_SUBMIT_SEMANTICS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_STRUCTURED_STATUS_SEMANTICS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_STRUCTURED_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_CONTENT_JSON_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_CONTENT_PUBLIC_SHAPE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_CONTENT_SUBMIT_SEMANTICS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_CONTENT_STATUS_SEMANTICS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_CONTENT_PROJECTION_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_PARITY_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_PUBLIC_STATUS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_DISPATCH_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_EFFECT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_COUNTERS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_EVIDENCE_REJECTED',
    'Wait-Task051McpServerNaturalExit',
    'TASK038_CURRENT_CODEX_TOOL_SERVER_EXIT_QUERY_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_SERVER_EXIT_TIMEOUT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_CLEANUP_REJECTED',
    '[mcp_servers.lattice.tools.lattice_task_submit]',
    '[mcp_servers.lattice.tools.lattice_task_status]',
    'enabled_tools = ["lattice_task_submit"]',
    'enabled_tools = ["lattice_task_status"]',
    'TASK051_CODEX_CALL_COUNT_PHASE_SELF_TEST=PASS',
    'TASK051_CODEX_EVENT_SUMMARY_SELF_TEST=PASS',
    'TASK051_CODEX_TOOL_FIELD_DIAGNOSTIC_SELF_TEST=PASS',
    'TASK051_CODEX_PHASE_TOOL_NO_MATERIALIZATION_SELF_TEST=PASS',
    'TASK051_CODEX_PER_TOOL_APPROVAL_SELF_TEST=PASS',
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

$currentCodexIdentityStart = $runnerSource.IndexOf('function Get-Task051CurrentCodexFileIdentity', [StringComparison]::Ordinal)
$currentCodexIdentityEnd = $runnerSource.IndexOf('function Assert-Task051NoReparseAncestor', $currentCodexIdentityStart, [StringComparison]::Ordinal)
if ($currentCodexIdentityStart -lt 0 -or $currentCodexIdentityEnd -le $currentCodexIdentityStart) {
    throw 'TASK051_CURRENT_CODEX_IDENTITY_SHAPE_REJECTED'
}
$currentCodexIdentitySource = $runnerSource.Substring($currentCodexIdentityStart, $currentCodexIdentityEnd - $currentCodexIdentityStart)
$currentCodexPathIndex = $currentCodexIdentitySource.IndexOf('$currentCodex = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA $script:Task051ExpectedCurrentCodexRelativePath))', [StringComparison]::Ordinal)
$currentCodexBoundaryIndex = $currentCodexIdentitySource.IndexOf('Assert-Task051NoReparseAncestor -Path $currentCodex -Boundary $currentCodexBoundary', [StringComparison]::Ordinal)
$currentCodexRegularFileIndex = $currentCodexIdentitySource.IndexOf("Assert-Task051RegularFile -Path `$currentCodex -FailureCode 'TASK051_CURRENT_CODEX_REJECTED'", [StringComparison]::Ordinal)
$currentCodexShaIndex = $currentCodexIdentitySource.IndexOf('$currentCodexSha256 = Get-Task051Sha256 -Path $currentCodex', [StringComparison]::Ordinal)
$currentCodexShaGuardIndex = $currentCodexIdentitySource.IndexOf('$currentCodexSha256 -cne $script:Task051ExpectedCurrentCodexSha256', [StringComparison]::Ordinal)
$currentCodexVersionIndex = $currentCodexIdentitySource.IndexOf('$currentCodexVersion -cne $script:Task051ExpectedCurrentCodexVersion', [StringComparison]::Ordinal)
$currentCodexUserAgentIndex = $currentCodexIdentitySource.IndexOf("`$currentCodexUserAgent = 'lattice-task051-acceptance/' + `$currentCodexSemanticVersion", [StringComparison]::Ordinal)
if (
    $currentCodexPathIndex -lt 0 -or
    $currentCodexBoundaryIndex -le $currentCodexPathIndex -or
    $currentCodexRegularFileIndex -le $currentCodexBoundaryIndex -or
    $currentCodexShaIndex -le $currentCodexRegularFileIndex -or
    $currentCodexShaGuardIndex -le $currentCodexShaIndex -or
    $currentCodexVersionIndex -le $currentCodexShaGuardIndex -or
    $currentCodexUserAgentIndex -le $currentCodexVersionIndex -or
    $currentCodexIdentitySource.IndexOf('Get-ChildItem', [StringComparison]::Ordinal) -ge 0 -or
    $runnerSource.IndexOf('codex-cli 0.147.0-alpha.6.6', [StringComparison]::Ordinal) -ge 0 -or
    [regex]::Matches($runnerSource, [regex]::Escape('Get-Task051CurrentCodexFileIdentity')).Count -ne 4
) {
    throw 'TASK051_CURRENT_CODEX_IDENTITY_SHAPE_REJECTED'
}

foreach ($identityBoundInvocation in @(
    [pscustomobject]@{ Start = 'function Invoke-Task051CodexDiscovery'; End = 'function Get-Task051ExecStructuredContent' },
    [pscustomobject]@{ Start = 'function Invoke-Task051CodexTool'; End = 'function Replace-Task051Exact' }
)) {
    $identityInvocationStart = $runnerSource.IndexOf([string]$identityBoundInvocation.Start, [StringComparison]::Ordinal)
    $identityInvocationEnd = $runnerSource.IndexOf([string]$identityBoundInvocation.End, $identityInvocationStart, [StringComparison]::Ordinal)
    if ($identityInvocationStart -lt 0 -or $identityInvocationEnd -le $identityInvocationStart) {
        throw 'TASK051_CURRENT_CODEX_PROCESS_IDENTITY_SHAPE_REJECTED'
    }
    $identityInvocationSource = $runnerSource.Substring($identityInvocationStart, $identityInvocationEnd - $identityInvocationStart)
    $identityRecheckIndex = $identityInvocationSource.IndexOf('$currentCodexIdentity = Get-Task051CurrentCodexFileIdentity', [StringComparison]::Ordinal)
    $nativeIdentityIndex = $identityInvocationSource.IndexOf('$currentCodexNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $codex -Directory $false', [StringComparison]::Ordinal)
    $processStartIndex = $identityInvocationSource.IndexOf('$owned = Start-Task051OwnedProcess -StartInfo $info', [StringComparison]::Ordinal)
    $processEvidenceIndex = $identityInvocationSource.IndexOf('$codexProcessEvidence = Get-Task051OwnedProcessEvidence', [StringComparison]::Ordinal)
    $codexAuthorityIndex = $identityInvocationSource.IndexOf('$codexProcessAuthority = $codexProcessEvidence.Authority', [StringComparison]::Ordinal)
    $cleanupBindingIndex = $identityInvocationSource.IndexOf('-CodexAuthority $codexProcessAuthority', [StringComparison]::Ordinal)
    if (
        $identityRecheckIndex -lt 0 -or
        $nativeIdentityIndex -le $identityRecheckIndex -or
        $processStartIndex -le $nativeIdentityIndex -or
        $processEvidenceIndex -le $processStartIndex -or
        $codexAuthorityIndex -le $processEvidenceIndex -or
        $cleanupBindingIndex -le $codexAuthorityIndex -or
        $identityInvocationSource.IndexOf('-ExpectedExecutableSha256 $script:Task051ExpectedCurrentCodexSha256', [StringComparison]::Ordinal) -lt 0 -or
        $identityInvocationSource.IndexOf('-ExpectedExecutableNativeIdentity $currentCodexNativeIdentity', [StringComparison]::Ordinal) -lt 0
    ) {
        throw 'TASK051_CURRENT_CODEX_PROCESS_IDENTITY_SHAPE_REJECTED'
    }
}

$codexHomeStart = $runnerSource.IndexOf('function New-Task051CodexHome', [StringComparison]::Ordinal)
$codexHomeEnd = $runnerSource.IndexOf('function Remove-Task051CodexCredential', $codexHomeStart, [StringComparison]::Ordinal)
if ($codexHomeStart -lt 0 -or $codexHomeEnd -le $codexHomeStart) {
    throw 'TASK051_CODEX_PER_TOOL_APPROVAL_SHAPE_REJECTED'
}
$codexHomeSource = $runnerSource.Substring($codexHomeStart, $codexHomeEnd - $codexHomeStart)
$approvalPolicySourceIndex = $codexHomeSource.IndexOf("'approval_policy = `"never`"'", [StringComparison]::Ordinal)
$sandboxModeSourceIndex = $codexHomeSource.IndexOf("'sandbox_mode = `"read-only`"'", [StringComparison]::Ordinal)
$serverApprovalSourceIndex = $codexHomeSource.IndexOf("'[mcp_servers.lattice]'", [StringComparison]::Ordinal)
$submitApprovalSourceIndex = $codexHomeSource.IndexOf("'[mcp_servers.lattice.tools.lattice_task_submit]'", [StringComparison]::Ordinal)
$statusApprovalSourceIndex = $codexHomeSource.IndexOf("'[mcp_servers.lattice.tools.lattice_task_status]'", [StringComparison]::Ordinal)
$discoveryPhaseSourceIndex = $codexHomeSource.IndexOf("'discovery' { `$null; break }", [StringComparison]::Ordinal)
$submitPhaseSourceIndex = $codexHomeSource.IndexOf("'submit' { 'enabled_tools = [`"lattice_task_submit`"]'; break }", [StringComparison]::Ordinal)
$preStatusPhaseSourceIndex = $codexHomeSource.IndexOf("'status-pre-restart' { 'enabled_tools = [`"lattice_task_status`"]'; break }", [StringComparison]::Ordinal)
$postStatusPhaseSourceIndex = $codexHomeSource.IndexOf("'status-post-restart' { 'enabled_tools = [`"lattice_task_status`"]'; break }", [StringComparison]::Ordinal)
$phaseRejectSourceIndex = $codexHomeSource.IndexOf("default { throw 'TASK051_CODEX_PHASE_REJECTED' }", [StringComparison]::Ordinal)
if (
    $approvalPolicySourceIndex -lt 0 -or
    $sandboxModeSourceIndex -le $approvalPolicySourceIndex -or
    $serverApprovalSourceIndex -le $sandboxModeSourceIndex -or
    $submitApprovalSourceIndex -le $serverApprovalSourceIndex -or
    $statusApprovalSourceIndex -le $submitApprovalSourceIndex -or
    [regex]::Matches($codexHomeSource, [regex]::Escape("'approval_policy = `"never`"'" )).Count -ne 1 -or
    [regex]::Matches($codexHomeSource, [regex]::Escape("'sandbox_mode = `"read-only`"'" )).Count -ne 1 -or
    [regex]::Matches($codexHomeSource, [regex]::Escape("'[mcp_servers.lattice.tools.lattice_task_submit]'" )).Count -ne 1 -or
    [regex]::Matches($codexHomeSource, [regex]::Escape("'[mcp_servers.lattice.tools.lattice_task_status]'" )).Count -ne 1 -or
    [regex]::Matches($codexHomeSource, [regex]::Escape("'approval_mode = `"approve`"'" )).Count -ne 2 -or
    $discoveryPhaseSourceIndex -lt 0 -or
    $submitPhaseSourceIndex -le $discoveryPhaseSourceIndex -or
    $preStatusPhaseSourceIndex -le $submitPhaseSourceIndex -or
    $postStatusPhaseSourceIndex -le $preStatusPhaseSourceIndex -or
    $phaseRejectSourceIndex -le $postStatusPhaseSourceIndex -or
    [regex]::Matches($codexHomeSource, [regex]::Escape("'enabled_tools = [`"lattice_task_submit`"]'" )).Count -ne 1 -or
    [regex]::Matches($codexHomeSource, [regex]::Escape("'enabled_tools = [`"lattice_task_status`"]'" )).Count -ne 2 -or
    $codexHomeSource.IndexOf('Set-Task051OwnerOnlyAcl -Path $configPath -Directory $false', [StringComparison]::Ordinal) -lt 0 -or
    $codexHomeSource.IndexOf('default_tools_approval_mode', [StringComparison]::Ordinal) -ge 0 -or
    $codexHomeSource.IndexOf('disabled_tools', [StringComparison]::Ordinal) -ge 0 -or
    $codexHomeSource.IndexOf('enabled_tools = ["lattice_delivery_', [StringComparison]::Ordinal) -ge 0 -or
    $codexHomeSource.IndexOf('[mcp_servers.lattice.tools.lattice_delivery_', [StringComparison]::Ordinal) -ge 0 -or
    $codexHomeSource.IndexOf('approval_mode = "auto"', [StringComparison]::Ordinal) -ge 0
) {
    throw 'TASK051_CODEX_PER_TOOL_APPROVAL_SHAPE_REJECTED'
}

$discoveryStart = $runnerSource.IndexOf('function Invoke-Task051CodexDiscovery', [StringComparison]::Ordinal)
$discoveryEnd = $runnerSource.IndexOf('function Get-Task051ExecStructuredContent', $discoveryStart, [StringComparison]::Ordinal)
if ($discoveryStart -lt 0 -or $discoveryEnd -le $discoveryStart) {
    throw 'TASK051_APP_SERVER_PROCESS_AUTHORITY_SHAPE_REJECTED'
}
$discoverySource = $runnerSource.Substring($discoveryStart, $discoveryEnd - $discoveryStart)
$processLifetimeStart = $runnerSource.IndexOf('function Test-Task051ProcessLifetime', [StringComparison]::Ordinal)
$ownedProcessEvidenceStart = $runnerSource.IndexOf('function Get-Task051OwnedProcessEvidence', $processLifetimeStart, [StringComparison]::Ordinal)
$ownedProcessEvidenceEnd = $runnerSource.IndexOf('function New-Task051CodexHome', $ownedProcessEvidenceStart, [StringComparison]::Ordinal)
if ($processLifetimeStart -lt 0 -or $ownedProcessEvidenceStart -le $processLifetimeStart -or $ownedProcessEvidenceEnd -le $ownedProcessEvidenceStart) {
    throw 'TASK051_APP_SERVER_PROCESS_LIFETIME_SHAPE_REJECTED'
}
$processLifetimeSource = $runnerSource.Substring($processLifetimeStart, $ownedProcessEvidenceStart - $processLifetimeStart)
$ownedProcessEvidenceSource = $runnerSource.Substring($ownedProcessEvidenceStart, $ownedProcessEvidenceEnd - $ownedProcessEvidenceStart)
foreach ($processLifetimeFragment in @(
    '$CreationFileTimeUtc -le $ObservedFileTimeFloorUtc',
    '$ObservedFileTimeCeilingUtc - $ObservedFileTimeFloorUtc -le 1',
    '$ExitFileTimeUtc -ge $ObservedFileTimeCeilingUtc',
    '$WasRunningAtCapture -and $ExitFileTimeUtc -eq 0',
    '-not $WasRunningAtCapture'
)) {
    if ($processLifetimeSource.IndexOf($processLifetimeFragment, [StringComparison]::Ordinal) -lt 0) {
        throw 'TASK051_APP_SERVER_PROCESS_LIFETIME_SHAPE_REJECTED'
    }
}
if (
    $ownedProcessEvidenceSource.IndexOf('TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_LIFETIME_REJECTED', [StringComparison]::Ordinal) -lt 0 -or
    $ownedProcessEvidenceSource.IndexOf('.IsAlive()', [StringComparison]::Ordinal) -ge 0 -or
    $processLifetimeSource.IndexOf('GetProcessTimes', [StringComparison]::Ordinal) -ge 0
) {
    throw 'TASK051_APP_SERVER_PROCESS_LIFETIME_SHAPE_REJECTED'
}
$sessionOpenReaderStart = $runnerSource.IndexOf('function Read-Task051McpSessionOpen', [StringComparison]::Ordinal)
$sessionOpenReaderEnd = $runnerSource.IndexOf('function Test-Task051McpSessionOpenReady', $sessionOpenReaderStart, [StringComparison]::Ordinal)
if ($sessionOpenReaderStart -lt 0 -or $sessionOpenReaderEnd -le $sessionOpenReaderStart) {
    throw 'TASK051_APP_SERVER_SESSION_OPEN_PARSE_DIAGNOSTIC_SHAPE_REJECTED'
}
$sessionOpenReaderSource = $runnerSource.Substring($sessionOpenReaderStart, $sessionOpenReaderEnd - $sessionOpenReaderStart)
$sessionOpenParseLeaves = @(
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_SOURCE_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_READ_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_FRAMING_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_JSON_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_KEYS_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_FIELDS_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_HASH_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_PROJECTION_REJECTED'
)
$sessionOpenParseLeafIndex = -1
foreach ($sessionOpenParseLeaf in $sessionOpenParseLeaves) {
    $quotedSessionOpenParseLeaf = "'" + $sessionOpenParseLeaf + "'"
    if (
        [regex]::Matches($sessionOpenReaderSource, [regex]::Escape($quotedSessionOpenParseLeaf)).Count -ne 1 -or
        [regex]::Matches($discoverySource, [regex]::Escape($quotedSessionOpenParseLeaf)).Count -ne 1
    ) {
        throw 'TASK051_APP_SERVER_SESSION_OPEN_PARSE_DIAGNOSTIC_SHAPE_REJECTED'
    }
    $nextSessionOpenParseLeafIndex = $sessionOpenReaderSource.IndexOf($quotedSessionOpenParseLeaf, [StringComparison]::Ordinal)
    if ($nextSessionOpenParseLeafIndex -le $sessionOpenParseLeafIndex) {
        throw 'TASK051_APP_SERVER_SESSION_OPEN_PARSE_DIAGNOSTIC_SHAPE_REJECTED'
    }
    $sessionOpenParseLeafIndex = $nextSessionOpenParseLeafIndex
}
if (
    [regex]::Matches($sessionOpenReaderSource, [regex]::Escape('[switch]$DetailedFailure')).Count -ne 1 -or
    [regex]::Matches($discoverySource, [regex]::Escape('-DetailedFailure')).Count -ne 1 -or
    [regex]::Matches($sessionOpenReaderSource, [regex]::Escape('[IO.FileStream]::new($canonicalPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, $share)')).Count -ne 1 -or
    [regex]::Matches($sessionOpenReaderSource, [regex]::Escape('[byte[]]::new(65537)')).Count -ne 1 -or
    [regex]::Matches($sessionOpenReaderSource, [regex]::Escape('$readStream.Length -ne $byteCount')).Count -ne 1 -or
    [regex]::Matches($sessionOpenReaderSource, [regex]::Escape('$readStream.Dispose()')).Count -ne 1 -or
    $sessionOpenReaderSource.IndexOf('[IO.File]::ReadAllBytes(', [StringComparison]::Ordinal) -ge 0
) {
    throw 'TASK051_APP_SERVER_SESSION_OPEN_PARSE_DIAGNOSTIC_SHAPE_REJECTED'
}
$pollActionCall = $discoverySource.IndexOf('-PollAction $sessionOpenPoll', [StringComparison]::Ordinal)
$authorityCapture = $discoverySource.IndexOf('$serverAuthority = $processEvidence.Authority', [StringComparison]::Ordinal)
if ($pollActionCall -lt 0 -or $authorityCapture -le $pollActionCall) {
    throw 'TASK051_APP_SERVER_PROCESS_AUTHORITY_SHAPE_REJECTED'
}
$sessionOpenLeafIndex = -1
foreach ($sessionOpenLeaf in @(
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_READY_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_PARSE_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_POLL_RESULT_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_REPLAY_READ_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_REPLAY_MISMATCH_REJECTED'
)) {
    if ([regex]::Matches($discoverySource, [regex]::Escape($sessionOpenLeaf)).Count -ne 1) {
        throw 'TASK051_APP_SERVER_SESSION_OPEN_DIAGNOSTIC_SHAPE_REJECTED'
    }
    $nextSessionOpenLeafIndex = $discoverySource.IndexOf($sessionOpenLeaf, [StringComparison]::Ordinal)
    if ($nextSessionOpenLeafIndex -le $sessionOpenLeafIndex) {
        throw 'TASK051_APP_SERVER_SESSION_OPEN_DIAGNOSTIC_SHAPE_REJECTED'
    }
    $sessionOpenLeafIndex = $nextSessionOpenLeafIndex
}
foreach ($forbiddenDiscoveryFragment in @('ParentProcessId =', '.ExecutablePath')) {
    if ($discoverySource.IndexOf($forbiddenDiscoveryFragment, [StringComparison]::Ordinal) -ge 0) {
        throw ('TASK051_APP_SERVER_PROCESS_AUTHORITY_SHAPE_REJECTED|' + $forbiddenDiscoveryFragment)
    }
}
if (
    [regex]::Matches($runnerSource, [regex]::Escape('::Acquire(')).Count -ne 1 -or
    $runnerSource.IndexOf('::Inspect(', [StringComparison]::Ordinal) -ge 0 -or
    $runnerSource.IndexOf('Get-Process -Id $ProcessId', [StringComparison]::Ordinal) -ge 0
) {
    throw 'TASK051_APP_SERVER_PROCESS_AUTHORITY_SHAPE_REJECTED'
}
$cleanupStart = $runnerSource.IndexOf('function Complete-Task051InvocationCleanup', [StringComparison]::Ordinal)
$cleanupEnd = $runnerSource.IndexOf('function Get-Task051McpEnvironment', $cleanupStart, [StringComparison]::Ordinal)
if ($cleanupStart -lt 0 -or $cleanupEnd -le $cleanupStart) {
    throw 'TASK051_APP_SERVER_PROCESS_AUTHORITY_CLEANUP_SHAPE_REJECTED'
}
$cleanupSource = $runnerSource.Substring($cleanupStart, $cleanupEnd - $cleanupStart)
$cleanupStop = $cleanupSource.IndexOf('Stop-Task051OwnedProcess -Owned $Owned', [StringComparison]::Ordinal)
$cleanupAlive = $cleanupSource.IndexOf('$ServerAuthority.IsAlive()', [StringComparison]::Ordinal)
$cleanupClose = $cleanupSource.IndexOf('$ServerAuthority.CloseExact()', [StringComparison]::Ordinal)
$cleanupCredential = $cleanupSource.IndexOf('Remove-Task051CodexCredential -CodexHome $CodexHome', [StringComparison]::Ordinal)
if (
    $cleanupStop -lt 0 -or
    $cleanupAlive -le $cleanupStop -or
    $cleanupClose -le $cleanupAlive -or
    $cleanupCredential -le $cleanupClose -or
    [regex]::Matches($cleanupSource, [regex]::Escape('$ServerAuthority.IsAlive()')).Count -ne 1
) {
    throw 'TASK051_APP_SERVER_PROCESS_AUTHORITY_CLEANUP_SHAPE_REJECTED'
}

$naturalExitStart = $runnerSource.IndexOf('function Wait-Task051McpServerNaturalExit', [StringComparison]::Ordinal)
$naturalExitEnd = $runnerSource.IndexOf('function Get-Task051McpEnvironment', $naturalExitStart, [StringComparison]::Ordinal)
if ($naturalExitStart -lt 0 -or $naturalExitEnd -le $naturalExitStart) {
    throw 'TASK051_MCP_SERVER_NATURAL_EXIT_SHAPE_REJECTED'
}
$naturalExitSource = $runnerSource.Substring($naturalExitStart, $naturalExitEnd - $naturalExitStart)
$naturalExitOpenIndex = $naturalExitSource.IndexOf('[Diagnostics.Process]::GetProcessById($ProcessId)', [StringComparison]::Ordinal)
$naturalExitWaitIndex = $naturalExitSource.IndexOf('$candidate.WaitForExit($TimeoutMilliseconds)', [StringComparison]::Ordinal)
$naturalExitDisposeIndex = $naturalExitSource.IndexOf('$candidate.Dispose()', [StringComparison]::Ordinal)
if (
    $naturalExitOpenIndex -lt 0 -or
    $naturalExitWaitIndex -le $naturalExitOpenIndex -or
    $naturalExitDisposeIndex -le $naturalExitWaitIndex -or
    [regex]::Matches($naturalExitSource, 'catch\s+\[ArgumentException\]\s*\{\s*return\s*\}').Count -ne 1 -or
    [regex]::Matches($naturalExitSource, [regex]::Escape('TASK038_CURRENT_CODEX_TOOL_SERVER_EXIT_QUERY_REJECTED')).Count -ne 3 -or
    [regex]::Matches($naturalExitSource, [regex]::Escape('TASK038_CURRENT_CODEX_TOOL_SERVER_EXIT_TIMEOUT_REJECTED')).Count -ne 1 -or
    $naturalExitSource.IndexOf('Start-Sleep', [StringComparison]::OrdinalIgnoreCase) -ge 0
) {
    throw 'TASK051_MCP_SERVER_NATURAL_EXIT_SHAPE_REJECTED'
}

$codexToolResolverStart = $runnerSource.IndexOf('function Resolve-Task051CodexToolFailure', [StringComparison]::Ordinal)
$codexToolInvokeStart = $runnerSource.IndexOf('function Invoke-Task051CodexTool', $codexToolResolverStart, [StringComparison]::Ordinal)
$codexToolInvokeEnd = $runnerSource.IndexOf('function Replace-Task051Exact', $codexToolInvokeStart, [StringComparison]::Ordinal)
if ($codexToolResolverStart -lt 0 -or $codexToolInvokeStart -le $codexToolResolverStart -or $codexToolInvokeEnd -le $codexToolInvokeStart) {
    throw 'TASK051_CODEX_TOOL_FAILURE_CLASSIFIER_SHAPE_REJECTED'
}
$codexToolResolverSource = $runnerSource.Substring($codexToolResolverStart, $codexToolInvokeStart - $codexToolResolverStart)
$codexToolInvokeSource = $runnerSource.Substring($codexToolInvokeStart, $codexToolInvokeEnd - $codexToolInvokeStart)
$codexPhaseToolStart = $runnerSource.IndexOf('function Assert-Task051CodexPhaseTool', $codexToolResolverStart, [StringComparison]::Ordinal)
if ($codexPhaseToolStart -lt 0 -or $codexToolInvokeStart -le $codexPhaseToolStart) {
    throw 'TASK051_CODEX_PHASE_TOOL_SHAPE_REJECTED'
}
$codexPhaseToolSource = $runnerSource.Substring($codexPhaseToolStart, $codexToolInvokeStart - $codexPhaseToolStart)
if (
    $codexPhaseToolSource.IndexOf("'submit' { 'lattice_task_submit'; break }", [StringComparison]::Ordinal) -lt 0 -or
    $codexPhaseToolSource.IndexOf("'status-pre-restart' { 'lattice_task_status'; break }", [StringComparison]::Ordinal) -lt 0 -or
    $codexPhaseToolSource.IndexOf("'status-post-restart' { 'lattice_task_status'; break }", [StringComparison]::Ordinal) -lt 0 -or
    $codexPhaseToolSource.IndexOf("default { throw 'TASK051_CODEX_PHASE_TOOL_REJECTED' }", [StringComparison]::Ordinal) -lt 0 -or
    $codexPhaseToolSource.IndexOf("if (`$Tool -cne `$expectedTool) { throw 'TASK051_CODEX_PHASE_TOOL_REJECTED' }", [StringComparison]::Ordinal) -lt 0
) {
    throw 'TASK051_CODEX_PHASE_TOOL_SHAPE_REJECTED'
}
$codexToolStructuredStart = $runnerSource.IndexOf('function Get-Task051ExecStructuredContent', [StringComparison]::Ordinal)
if ($codexToolStructuredStart -lt 0 -or $codexToolResolverStart -le $codexToolStructuredStart) {
    throw 'TASK051_CODEX_TOOL_IDENTITY_SPLIT_SHAPE_REJECTED'
}
$codexToolStructuredSource = $runnerSource.Substring($codexToolStructuredStart, $codexToolResolverStart - $codexToolStructuredStart)
$codexEventSummaryStart = $runnerSource.IndexOf('function Get-Task051CodexEventSummary', [StringComparison]::Ordinal)
$codexPublicProjectionStart = $runnerSource.IndexOf('function Resolve-Task051PublicProjectionFailure', $codexEventSummaryStart, [StringComparison]::Ordinal)
if ($codexEventSummaryStart -lt 0 -or $codexPublicProjectionStart -le $codexEventSummaryStart -or $codexToolStructuredStart -le $codexPublicProjectionStart) {
    throw 'TASK051_CODEX_EVENT_SUMMARY_SHAPE_REJECTED'
}
$codexEventSummarySource = $runnerSource.Substring($codexEventSummaryStart, $codexPublicProjectionStart - $codexEventSummaryStart)
$codexPublicProjectionSource = $runnerSource.Substring($codexPublicProjectionStart, $codexToolStructuredStart - $codexPublicProjectionStart)
$codexEventSummaryKeys = @(
    'schema_version', 'phase', 'expected_tool', 'total_event_count', 'mcp_started_count',
    'mcp_completed_count', 'expected_started_count', 'expected_completed_count', 'other_completed_count',
    'completed_status_count', 'failed_status_count', 'unknown_status_count', 'agent_message_completed_count',
    'turn_completed_count', 'response_completed_count'
)
$codexEventSummaryKeyIndex = -1
foreach ($codexEventSummaryKey in $codexEventSummaryKeys) {
    $nextCodexEventSummaryKeyIndex = $codexEventSummarySource.IndexOf($codexEventSummaryKey, $codexEventSummaryKeyIndex + 1, [StringComparison]::Ordinal)
    if ($nextCodexEventSummaryKeyIndex -le $codexEventSummaryKeyIndex) {
        throw 'TASK051_CODEX_EVENT_SUMMARY_SHAPE_REJECTED'
    }
    $codexEventSummaryKeyIndex = $nextCodexEventSummaryKeyIndex
}
foreach ($forbiddenSummaryFragment in @('.arguments', '.result', '.error', '.text', 'prompt', 'environment')) {
    if ($codexEventSummarySource.IndexOf($forbiddenSummaryFragment, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
        throw 'TASK051_CODEX_EVENT_SUMMARY_SHAPE_REJECTED'
    }
}
$codexCallCountRawLeaves = @(
    'TASK051_CODEX_SUBMIT_CALL_COUNT_ZERO_REJECTED',
    'TASK051_CODEX_STATUS_PRE_RESTART_CALL_COUNT_ZERO_REJECTED',
    'TASK051_CODEX_STATUS_POST_RESTART_CALL_COUNT_ZERO_REJECTED',
    'TASK051_CODEX_SUBMIT_CALL_COUNT_MULTIPLE_REJECTED',
    'TASK051_CODEX_STATUS_PRE_RESTART_CALL_COUNT_MULTIPLE_REJECTED',
    'TASK051_CODEX_STATUS_POST_RESTART_CALL_COUNT_MULTIPLE_REJECTED'
)
$codexCallCountMappedLeaves = @(
    'TASK038_CURRENT_CODEX_TOOL_SUBMIT_CALL_COUNT_ZERO_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_PRE_RESTART_CALL_COUNT_ZERO_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_POST_RESTART_CALL_COUNT_ZERO_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_SUBMIT_CALL_COUNT_MULTIPLE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_PRE_RESTART_CALL_COUNT_MULTIPLE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_POST_RESTART_CALL_COUNT_MULTIPLE_REJECTED'
)
$codexCallCountRawIndex = -1
for ($codexCallCountLeafIndex = 0; $codexCallCountLeafIndex -lt $codexCallCountRawLeaves.Count; $codexCallCountLeafIndex++) {
    $rawCallCountLeaf = $codexCallCountRawLeaves[$codexCallCountLeafIndex]
    if ([regex]::Matches($codexToolStructuredSource, [regex]::Escape("'" + $rawCallCountLeaf + "'")).Count -ne 1) {
        throw 'TASK051_CODEX_CALL_COUNT_PHASE_SHAPE_REJECTED'
    }
    $nextCodexCallCountRawIndex = $codexToolStructuredSource.IndexOf($rawCallCountLeaf, $codexCallCountRawIndex + 1, [StringComparison]::Ordinal)
    if ($nextCodexCallCountRawIndex -le $codexCallCountRawIndex) {
        throw 'TASK051_CODEX_CALL_COUNT_PHASE_SHAPE_REJECTED'
    }
    $codexCallCountRawIndex = $nextCodexCallCountRawIndex
    $callCountMapping = "'" + $rawCallCountLeaf + "' { return '" + $codexCallCountMappedLeaves[$codexCallCountLeafIndex] + "' }"
    if ([regex]::Matches($codexToolResolverSource, [regex]::Escape($callCountMapping)).Count -ne 1) {
        throw 'TASK051_CODEX_CALL_COUNT_PHASE_SHAPE_REJECTED'
    }
}
$callCountMappingOrder = @(
    "'TASK051_CODEX_SUBMIT_CALL_COUNT_ZERO_REJECTED' { return 'TASK038_CURRENT_CODEX_TOOL_SUBMIT_CALL_COUNT_ZERO_REJECTED' }",
    "'TASK051_CODEX_SUBMIT_CALL_COUNT_MULTIPLE_REJECTED' { return 'TASK038_CURRENT_CODEX_TOOL_SUBMIT_CALL_COUNT_MULTIPLE_REJECTED' }",
    "'TASK051_CODEX_STATUS_PRE_RESTART_CALL_COUNT_ZERO_REJECTED' { return 'TASK038_CURRENT_CODEX_TOOL_STATUS_PRE_RESTART_CALL_COUNT_ZERO_REJECTED' }",
    "'TASK051_CODEX_STATUS_PRE_RESTART_CALL_COUNT_MULTIPLE_REJECTED' { return 'TASK038_CURRENT_CODEX_TOOL_STATUS_PRE_RESTART_CALL_COUNT_MULTIPLE_REJECTED' }",
    "'TASK051_CODEX_STATUS_POST_RESTART_CALL_COUNT_ZERO_REJECTED' { return 'TASK038_CURRENT_CODEX_TOOL_STATUS_POST_RESTART_CALL_COUNT_ZERO_REJECTED' }",
    "'TASK051_CODEX_STATUS_POST_RESTART_CALL_COUNT_MULTIPLE_REJECTED' { return 'TASK038_CURRENT_CODEX_TOOL_STATUS_POST_RESTART_CALL_COUNT_MULTIPLE_REJECTED' }",
    "'TASK051_CODEX_EVENT_SUMMARY_REJECTED' { return 'TASK038_CURRENT_CODEX_TOOL_EVENT_SUMMARY_REJECTED' }"
)
$callCountMappingOrderIndex = -1
foreach ($orderedCallCountMapping in $callCountMappingOrder) {
    $nextCallCountMappingOrderIndex = $codexToolResolverSource.IndexOf($orderedCallCountMapping, $callCountMappingOrderIndex + 1, [StringComparison]::Ordinal)
    if ($nextCallCountMappingOrderIndex -le $callCountMappingOrderIndex) {
        throw 'TASK051_CODEX_CALL_COUNT_PHASE_SHAPE_REJECTED'
    }
    $callCountMappingOrderIndex = $nextCallCountMappingOrderIndex
}
if (
    $codexToolStructuredSource.IndexOf('TASK051_CODEX_SUBMIT_CALL_COUNT_REJECTED', [StringComparison]::Ordinal) -ge 0 -or
    $codexToolStructuredSource.IndexOf('TASK051_CODEX_STATUS_CALL_COUNT_REJECTED', [StringComparison]::Ordinal) -ge 0 -or
    $codexToolResolverSource.IndexOf('TASK038_CURRENT_CODEX_TOOL_CALL_COUNT_REJECTED', [StringComparison]::Ordinal) -ge 0
) {
    throw 'TASK051_CODEX_CALL_COUNT_PHASE_SHAPE_REJECTED'
}
$codexToolServerGate = $codexToolStructuredSource.IndexOf("throw 'TASK051_CODEX_TOOL_SERVER_REJECTED'", [StringComparison]::Ordinal)
$codexToolNameGate = $codexToolStructuredSource.IndexOf("throw 'TASK051_CODEX_TOOL_NAME_REJECTED'", [StringComparison]::Ordinal)
$codexToolStatusGate = $codexToolStructuredSource.IndexOf("throw 'TASK051_CODEX_TOOL_STATUS_REJECTED'", [StringComparison]::Ordinal)
if (
    $codexToolServerGate -lt 0 -or
    $codexToolNameGate -le $codexToolServerGate -or
    $codexToolStatusGate -le $codexToolNameGate -or
    [regex]::Matches($codexToolStructuredSource, 'TASK051_CODEX_TOOL_(?:SERVER|NAME|STATUS)_REJECTED').Count -ne 3 -or
    $codexToolStructuredSource.IndexOf('TASK051_CODEX_TOOL_IDENTITY_REJECTED', [StringComparison]::Ordinal) -ge 0 -or
    $codexToolStructuredSource.IndexOf("`$call.PSObject.Properties['error']", [StringComparison]::Ordinal) -lt 0 -or
    $codexToolStructuredSource.IndexOf("`$call.PSObject.Properties['result']", [StringComparison]::Ordinal) -lt 0 -or
    $codexToolStructuredSource.IndexOf('$call.error', [StringComparison]::Ordinal) -ge 0 -or
    $codexToolStructuredSource.IndexOf('$call.result', [StringComparison]::Ordinal) -ge 0
) {
    throw 'TASK051_CODEX_TOOL_IDENTITY_SPLIT_SHAPE_REJECTED'
}
$publicKindSubmitIndex = $codexToolStructuredSource.IndexOf("'lattice_task_submit' { 'SUBMIT'; break }", [StringComparison]::Ordinal)
$publicKindStatusIndex = $codexToolStructuredSource.IndexOf("'lattice_task_status' { 'STATUS'; break }", [StringComparison]::Ordinal)
$publicKindDefaultIndex = $codexToolStructuredSource.IndexOf("default { throw 'TASK051_CODEX_TOOL_PUBLIC_KIND_REJECTED' }", [StringComparison]::Ordinal)
$structuredPublicValidationIndex = $codexToolStructuredSource.IndexOf('Assert-Task051PublicStatus -Value $structured -Kind $publicKind -DetailedFailure', [StringComparison]::Ordinal)
$contentPublicValidationIndex = $codexToolStructuredSource.IndexOf('Assert-Task051PublicStatus -Value $contentValue -Kind $publicKind -DetailedFailure', [StringComparison]::Ordinal)
$parityValidationIndex = $codexToolStructuredSource.IndexOf('Assert-Task051SameStatus -Expected $structured -Actual $contentValue -Kind $publicKind', [StringComparison]::Ordinal)
if (
    $publicKindSubmitIndex -lt 0 -or
    $publicKindStatusIndex -le $publicKindSubmitIndex -or
    $publicKindDefaultIndex -le $publicKindStatusIndex -or
    $structuredPublicValidationIndex -le $publicKindDefaultIndex -or
    $contentPublicValidationIndex -le $structuredPublicValidationIndex -or
    $parityValidationIndex -le $contentPublicValidationIndex -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape('-Kind $publicKind')).Count -ne 5 -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape('-DetailedFailure')).Count -ne 2 -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape("'TASK051_CODEX_TOOL_PUBLIC_KIND_REJECTED'")).Count -ne 1 -or
    [regex]::Matches($codexToolResolverSource, [regex]::Escape("'TASK051_CODEX_TOOL_PUBLIC_KIND_REJECTED' { return 'TASK038_CURRENT_CODEX_TOOL_PUBLIC_KIND_REJECTED' }")).Count -ne 1 -or
    $codexToolStructuredSource.IndexOf("Assert-Task051PublicStatus -Value `$structured -Kind 'STATUS'", [StringComparison]::Ordinal) -ge 0 -or
    $codexToolStructuredSource.IndexOf("Assert-Task051PublicStatus -Value `$structured -Kind 'SUBMIT'", [StringComparison]::Ordinal) -ge 0 -or
    $codexToolStructuredSource.IndexOf("Assert-Task051PublicStatus -Value `$contentValue -Kind 'STATUS'", [StringComparison]::Ordinal) -ge 0 -or
    $codexToolStructuredSource.IndexOf("Assert-Task051PublicStatus -Value `$contentValue -Kind 'SUBMIT'", [StringComparison]::Ordinal) -ge 0
) {
    throw 'TASK051_CODEX_TOOL_PUBLIC_PROJECTION_SHAPE_REJECTED'
}
$projectionMappingFragments = @(
    "if (`$Message -ceq 'TASK051_PUBLIC_STATUS_SHAPE_REJECTED')",
    "'STRUCTURED' { return 'TASK051_CODEX_TOOL_RESULT_STRUCTURED_PUBLIC_SHAPE_REJECTED' }",
    "'CONTENT' { return 'TASK051_CODEX_TOOL_RESULT_CONTENT_PUBLIC_SHAPE_REJECTED' }"
)
$publicProjectionLeaves = @(
    'SCHEMA',
    'STATUS_NOT_SUBMITTED',
    'STATUS_RECONCILIATION_REQUIRED',
    'STATUS_FAILED',
    'STATUS_LOWER_COMPLETED',
    'STATUS_OTHER',
    'TASK_STATE',
    'TASK_REF',
    'LEDGER_HEAD',
    'RESULT_DIGEST'
)
foreach ($projectionName in @('STRUCTURED', 'CONTENT')) {
    foreach ($kindName in @('SUBMIT', 'STATUS')) {
        foreach ($fieldName in $publicProjectionLeaves) {
            $projectionMappingFragments += "'" + $projectionName + '|' + $kindName + '|TASK051_PUBLIC_STATUS_' + $fieldName + "_REJECTED' { return 'TASK051_CODEX_TOOL_RESULT_" + $projectionName + '_' + $kindName + '_' + $fieldName + "_REJECTED' }"
        }
    }
}
$projectionMappingFragments += @(
    "'STRUCTURED|SUBMIT|TASK051_SUBMIT_SEMANTICS_REJECTED' { return 'TASK051_CODEX_TOOL_RESULT_STRUCTURED_SUBMIT_SEMANTICS_REJECTED' }",
    "'STRUCTURED|STATUS|TASK051_STATUS_SEMANTICS_REJECTED' { return 'TASK051_CODEX_TOOL_RESULT_STRUCTURED_STATUS_SEMANTICS_REJECTED' }",
    "'CONTENT|SUBMIT|TASK051_SUBMIT_SEMANTICS_REJECTED' { return 'TASK051_CODEX_TOOL_RESULT_CONTENT_SUBMIT_SEMANTICS_REJECTED' }",
    "'CONTENT|STATUS|TASK051_STATUS_SEMANTICS_REJECTED' { return 'TASK051_CODEX_TOOL_RESULT_CONTENT_STATUS_SEMANTICS_REJECTED' }",
    "'STRUCTURED' { return 'TASK051_CODEX_TOOL_RESULT_STRUCTURED_REJECTED' }",
    "'CONTENT' { return 'TASK051_CODEX_TOOL_RESULT_CONTENT_PROJECTION_REJECTED' }"
)
$projectionMappingIndex = -1
foreach ($projectionMappingFragment in $projectionMappingFragments) {
    $nextProjectionMappingIndex = $codexPublicProjectionSource.IndexOf($projectionMappingFragment, $projectionMappingIndex + 1, [StringComparison]::Ordinal)
    if ($nextProjectionMappingIndex -le $projectionMappingIndex) {
        throw 'TASK051_CODEX_TOOL_PUBLIC_PROJECTION_SHAPE_REJECTED'
    }
    $projectionMappingIndex = $nextProjectionMappingIndex
}
if (
    $codexPublicProjectionSource.IndexOf('throw $Message', [StringComparison]::OrdinalIgnoreCase) -ge 0 -or
    $codexPublicProjectionSource.IndexOf('throw $_', [StringComparison]::OrdinalIgnoreCase) -ge 0
) {
    throw 'TASK051_CODEX_TOOL_PUBLIC_PROJECTION_SHAPE_REJECTED'
}
$codexToolResultRawLeaves = @(
    'TASK051_CODEX_TOOL_RESULT_ERROR_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_MISSING_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_KEYS_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_CONTENT_SHAPE_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_META_SHAPE_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_META_IDENTITY_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_STRUCTURED_NULL_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_STRUCTURED_PUBLIC_SHAPE_REJECTED'
)
foreach ($kindName in @('SUBMIT', 'STATUS')) {
    foreach ($fieldName in $publicProjectionLeaves) {
        $codexToolResultRawLeaves += 'TASK051_CODEX_TOOL_RESULT_STRUCTURED_' + $kindName + '_' + $fieldName + '_REJECTED'
    }
}
$codexToolResultRawLeaves += @(
    'TASK051_CODEX_TOOL_RESULT_STRUCTURED_SUBMIT_SEMANTICS_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_STRUCTURED_STATUS_SEMANTICS_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_CONTENT_JSON_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_CONTENT_PUBLIC_SHAPE_REJECTED'
)
foreach ($kindName in @('SUBMIT', 'STATUS')) {
    foreach ($fieldName in $publicProjectionLeaves) {
        $codexToolResultRawLeaves += 'TASK051_CODEX_TOOL_RESULT_CONTENT_' + $kindName + '_' + $fieldName + '_REJECTED'
    }
}
$codexToolResultRawLeaves += @(
    'TASK051_CODEX_TOOL_RESULT_CONTENT_SUBMIT_SEMANTICS_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_CONTENT_STATUS_SEMANTICS_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_STRUCTURED_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_CONTENT_PROJECTION_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_PARITY_REJECTED'
)
$codexToolParserRawLeaves = @(
    'TASK051_CODEX_TOOL_RESULT_ERROR_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_MISSING_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_KEYS_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_CONTENT_SHAPE_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_META_SHAPE_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_META_IDENTITY_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_STRUCTURED_NULL_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_CONTENT_JSON_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_PARITY_REJECTED'
)
foreach ($codexToolParserRawLeaf in $codexToolParserRawLeaves) {
    if ([regex]::Matches($codexToolStructuredSource, [regex]::Escape("'" + $codexToolParserRawLeaf + "'")).Count -ne 1) {
        throw 'TASK051_CODEX_TOOL_RESULT_SPLIT_SHAPE_REJECTED'
    }
}
$codexProjectionRawLeaves = @(
    'TASK051_CODEX_TOOL_RESULT_STRUCTURED_PUBLIC_SHAPE_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_CONTENT_PUBLIC_SHAPE_REJECTED'
)
foreach ($projectionName in @('STRUCTURED', 'CONTENT')) {
    foreach ($kindName in @('SUBMIT', 'STATUS')) {
        foreach ($fieldName in $publicProjectionLeaves) {
            $codexProjectionRawLeaves += 'TASK051_CODEX_TOOL_RESULT_' + $projectionName + '_' + $kindName + '_' + $fieldName + '_REJECTED'
        }
    }
}
$codexProjectionRawLeaves += @(
    'TASK051_CODEX_TOOL_RESULT_STRUCTURED_SUBMIT_SEMANTICS_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_STRUCTURED_STATUS_SEMANTICS_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_CONTENT_SUBMIT_SEMANTICS_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_CONTENT_STATUS_SEMANTICS_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_STRUCTURED_REJECTED',
    'TASK051_CODEX_TOOL_RESULT_CONTENT_PROJECTION_REJECTED'
)
$codexProjectionRawLeafIndex = -1
foreach ($codexProjectionRawLeaf in $codexProjectionRawLeaves) {
    if ([regex]::Matches($codexPublicProjectionSource, [regex]::Escape("'" + $codexProjectionRawLeaf + "'")).Count -ne 1) {
        throw 'TASK051_CODEX_TOOL_RESULT_SPLIT_SHAPE_REJECTED'
    }
    $nextCodexProjectionRawLeafIndex = $codexPublicProjectionSource.IndexOf($codexProjectionRawLeaf, $codexProjectionRawLeafIndex + 1, [StringComparison]::Ordinal)
    if ($nextCodexProjectionRawLeafIndex -le $codexProjectionRawLeafIndex) {
        throw 'TASK051_CODEX_TOOL_RESULT_SPLIT_SHAPE_REJECTED'
    }
    $codexProjectionRawLeafIndex = $nextCodexProjectionRawLeafIndex
}
if (
    $codexToolStructuredSource.IndexOf('TASK051_CODEX_TOOL_RESULT_REJECTED', [StringComparison]::Ordinal) -ge 0 -or
    $codexToolStructuredSource.IndexOf('TASK051_CODEX_TOOL_RESULT_ENVELOPE_REJECTED', [StringComparison]::Ordinal) -ge 0
) {
    throw 'TASK051_CODEX_TOOL_RESULT_SPLIT_SHAPE_REJECTED'
}
$codexToolResultMappedLeaves = @($codexToolResultRawLeaves | ForEach-Object { $_.Replace('TASK051_CODEX', 'TASK038_CURRENT_CODEX') })
$codexToolResultMappingIndex = -1
for ($resultLeafIndex = 0; $resultLeafIndex -lt $codexToolResultRawLeaves.Count; $resultLeafIndex++) {
    $mappingFragment = "'" + $codexToolResultRawLeaves[$resultLeafIndex] + "' { return '" + $codexToolResultMappedLeaves[$resultLeafIndex] + "' }"
    if ([regex]::Matches($codexToolResolverSource, [regex]::Escape($mappingFragment)).Count -ne 1) {
        throw 'TASK051_CODEX_TOOL_RESULT_SPLIT_SHAPE_REJECTED'
    }
    $nextCodexToolResultMappingIndex = $codexToolResolverSource.IndexOf($mappingFragment, $codexToolResultMappingIndex + 1, [StringComparison]::Ordinal)
    if ($nextCodexToolResultMappingIndex -le $codexToolResultMappingIndex) {
        throw 'TASK051_CODEX_TOOL_RESULT_SPLIT_SHAPE_REJECTED'
    }
    $codexToolResultMappingIndex = $nextCodexToolResultMappingIndex
}
if (
    $runnerSource.IndexOf('TASK051_CODEX_TOOL_RESULT_META_ABSENT_REJECTED', [StringComparison]::Ordinal) -ge 0 -or
    $runnerSource.IndexOf('TASK038_CURRENT_CODEX_TOOL_RESULT_META_ABSENT_REJECTED', [StringComparison]::Ordinal) -ge 0 -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape("'content,structured_content' { `$metaPresent = `$false; break }")).Count -ne 1 -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape("'_meta,content,structured_content' { `$metaPresent = `$true; break }")).Count -ne 1 -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape("`$metaProperty = `$result.PSObject.Properties['_meta']")).Count -ne 1 -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape("[string]`$serverInfo.Name -cne 'io.modelcontextprotocol/serverInfo'")).Count -ne 1 -or
    $codexToolStructuredSource.IndexOf('$result._meta', [StringComparison]::Ordinal) -ge 0 -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape("[ValidateSet('ABSENT', 'PRESENT_VERIFIED')]")).Count -ne 1 -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape("`$metaMode = 'ABSENT'")).Count -ne 1 -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape("`$metaMode = 'PRESENT_VERIFIED'")).Count -ne 1 -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape('MetaPresent = $metaPresent')).Count -ne 1 -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape('MetaSha256 = $metaSha256')).Count -ne 1 -or
    [regex]::Matches($codexToolStructuredSource, [regex]::Escape('lattice.task051.codex-result-meta-commitment.v1')).Count -ne 1
) {
    throw 'TASK051_CODEX_TOOL_RESULT_META_OPTIONAL_SHAPE_REJECTED'
}
foreach ($codexToolLeaf in @(
    'TASK038_CURRENT_CODEX_TOOL_HOME_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_START_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_PROCESS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_PROMPT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_WAIT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_EXIT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_OUTPUT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_EVENT_JSON_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_SUBMIT_CALL_COUNT_ZERO_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_SUBMIT_CALL_COUNT_MULTIPLE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_PRE_RESTART_CALL_COUNT_ZERO_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_PRE_RESTART_CALL_COUNT_MULTIPLE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_POST_RESTART_CALL_COUNT_ZERO_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_POST_RESTART_CALL_COUNT_MULTIPLE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_EVENT_SUMMARY_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_UNEXPECTED_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_SERVER_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_NAME_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_STATUS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_ARGUMENT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_ERROR_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_MISSING_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_KEYS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_CONTENT_SHAPE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_META_SHAPE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_META_IDENTITY_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_STRUCTURED_NULL_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_STRUCTURED_PUBLIC_SHAPE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_STRUCTURED_SUBMIT_SEMANTICS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_STRUCTURED_STATUS_SEMANTICS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_STRUCTURED_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_CONTENT_JSON_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_CONTENT_PUBLIC_SHAPE_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_CONTENT_SUBMIT_SEMANTICS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_CONTENT_STATUS_SEMANTICS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_CONTENT_PROJECTION_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_RESULT_PARITY_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_PUBLIC_STATUS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_DISPATCH_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_EFFECT_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_COUNTERS_REJECTED',
    'TASK038_CURRENT_CODEX_TOOL_EVIDENCE_REJECTED'
)) {
    if ($codexToolResolverSource.IndexOf($codexToolLeaf, [StringComparison]::Ordinal) -lt 0) {
        throw 'TASK051_CODEX_TOOL_FAILURE_CLASSIFIER_SHAPE_REJECTED'
    }
}
$codexToolStageIndex = -1
foreach ($codexToolStage in @('HOME', 'START', 'PROCESS', 'PROMPT', 'WAIT', 'EXIT', 'OUTPUT', 'EVENT_JSON', 'RESULT', 'PUBLIC_STATUS', 'DISPATCH', 'EFFECT', 'COUNTERS', 'EVIDENCE')) {
    $nextCodexToolStageIndex = $codexToolInvokeSource.IndexOf(("`$failureStage = '" + $codexToolStage + "'"), $codexToolStageIndex + 1, [StringComparison]::Ordinal)
    if ($nextCodexToolStageIndex -le $codexToolStageIndex) {
        throw 'TASK051_CODEX_TOOL_FAILURE_CLASSIFIER_SHAPE_REJECTED'
    }
    $codexToolStageIndex = $nextCodexToolStageIndex
}
$phaseToolAssertIndex = $codexToolInvokeSource.IndexOf('Assert-Task051CodexPhaseTool -Phase $Phase -Tool $Tool', [StringComparison]::Ordinal)
$acceptanceSinkCreateIndex = $codexToolInvokeSource.IndexOf('$acceptanceSink = New-Task038McpAcceptanceEvidenceSink', [StringComparison]::Ordinal)
$codexHomeCreateIndex = $codexToolInvokeSource.IndexOf('$codexHome = New-Task051CodexHome', [StringComparison]::Ordinal)
$structuredParserIndex = $codexToolInvokeSource.IndexOf('Get-Task051ExecStructuredContent -Events $events -Phase $Phase -Tool $Tool', [StringComparison]::Ordinal)
$structuredParserCatchIndex = $codexToolInvokeSource.IndexOf('catch {', $structuredParserIndex, [StringComparison]::Ordinal)
$callCountAllowListIndex = $codexToolInvokeSource.IndexOf('$resultFailure -in @(', $structuredParserCatchIndex, [StringComparison]::Ordinal)
$summaryWriterFragment = "Write-Task051JsonEvidence -Path (Join-Path `$EvidenceRoot ('task051-' + `$Phase + '-codex-event-summary.json')) -Value `$eventSummary"
$summaryWriterIndex = $codexToolInvokeSource.IndexOf($summaryWriterFragment, $callCountAllowListIndex, [StringComparison]::Ordinal)
$resultRethrowIndex = $codexToolInvokeSource.IndexOf('throw $resultFailure', $summaryWriterIndex, [StringComparison]::Ordinal)
$structuredProjectionIndex = $codexToolInvokeSource.IndexOf('$structured = $envelope.StructuredContent', $resultRethrowIndex, [StringComparison]::Ordinal)
$metaCommitmentIndex = $codexToolInvokeSource.IndexOf('$metaCommitmentSha256 = Get-Task051CodexResultMetaCommitment', $structuredProjectionIndex, [StringComparison]::Ordinal)
$toolEvidenceIndex = $codexToolInvokeSource.IndexOf("schema_version = 'lattice.task051.current-codex-tool-call.v2'", $metaCommitmentIndex, [StringComparison]::Ordinal)
$serverProcessProjectionIndex = $codexToolInvokeSource.IndexOf('$serverProcessId = [int](($firstDispatchLine | ConvertFrom-Json -ErrorAction Stop).process_id)', [StringComparison]::Ordinal)
$serverNaturalExitIndex = $codexToolInvokeSource.IndexOf('Wait-Task051McpServerNaturalExit -ProcessId $serverProcessId -TimeoutMilliseconds 30000', $serverProcessProjectionIndex, [StringComparison]::Ordinal)
$dispatchEvidenceIndex = $codexToolInvokeSource.IndexOf('$dispatch = Read-Task038McpAcceptanceEvidence', $serverNaturalExitIndex, [StringComparison]::Ordinal)
$effectEvidenceIndex = $codexToolInvokeSource.IndexOf('$effects = Read-Task038McpObservedEffectEvidence', $dispatchEvidenceIndex, [StringComparison]::Ordinal)
$serverAbsenceIndex = $codexToolInvokeSource.IndexOf('Get-Process -Id $serverProcessId -ErrorAction SilentlyContinue', $effectEvidenceIndex, [StringComparison]::Ordinal)
$callCountGuardSource = if ($callCountAllowListIndex -ge 0 -and $summaryWriterIndex -gt $callCountAllowListIndex) {
    $codexToolInvokeSource.Substring($callCountAllowListIndex, $summaryWriterIndex - $callCountAllowListIndex)
}
else { '' }
$callCountGuardLeaves = @(
    'TASK051_CODEX_SUBMIT_CALL_COUNT_ZERO_REJECTED',
    'TASK051_CODEX_SUBMIT_CALL_COUNT_MULTIPLE_REJECTED',
    'TASK051_CODEX_STATUS_PRE_RESTART_CALL_COUNT_ZERO_REJECTED',
    'TASK051_CODEX_STATUS_PRE_RESTART_CALL_COUNT_MULTIPLE_REJECTED',
    'TASK051_CODEX_STATUS_POST_RESTART_CALL_COUNT_ZERO_REJECTED',
    'TASK051_CODEX_STATUS_POST_RESTART_CALL_COUNT_MULTIPLE_REJECTED'
)
$callCountGuardLeafIndex = -1
foreach ($callCountGuardLeaf in $callCountGuardLeaves) {
    if ([regex]::Matches($callCountGuardSource, [regex]::Escape("'" + $callCountGuardLeaf + "'")).Count -ne 1) {
        throw 'TASK051_CODEX_CALL_COUNT_PHASE_SHAPE_REJECTED'
    }
    $nextCallCountGuardLeafIndex = $callCountGuardSource.IndexOf($callCountGuardLeaf, $callCountGuardLeafIndex + 1, [StringComparison]::Ordinal)
    if ($nextCallCountGuardLeafIndex -le $callCountGuardLeafIndex) {
        throw 'TASK051_CODEX_CALL_COUNT_PHASE_SHAPE_REJECTED'
    }
    $callCountGuardLeafIndex = $nextCallCountGuardLeafIndex
}
if (
    $phaseToolAssertIndex -lt 0 -or
    $acceptanceSinkCreateIndex -le $phaseToolAssertIndex -or
    $codexHomeCreateIndex -le $acceptanceSinkCreateIndex -or
    $structuredParserIndex -le $codexHomeCreateIndex -or
    $structuredParserCatchIndex -le $structuredParserIndex -or
    $callCountAllowListIndex -le $structuredParserCatchIndex -or
    $summaryWriterIndex -le $callCountAllowListIndex -or
    $resultRethrowIndex -le $summaryWriterIndex -or
    $structuredProjectionIndex -le $resultRethrowIndex -or
    $metaCommitmentIndex -le $structuredProjectionIndex -or
    $toolEvidenceIndex -le $metaCommitmentIndex -or
    $serverProcessProjectionIndex -le $structuredProjectionIndex -or
    $serverNaturalExitIndex -le $serverProcessProjectionIndex -or
    $dispatchEvidenceIndex -le $serverNaturalExitIndex -or
    $effectEvidenceIndex -le $dispatchEvidenceIndex -or
    $serverAbsenceIndex -le $effectEvidenceIndex -or
    [regex]::Matches($codexToolInvokeSource, [regex]::Escape('Wait-Task051McpServerNaturalExit -ProcessId $serverProcessId -TimeoutMilliseconds 30000')).Count -ne 1 -or
    [regex]::Matches($codexToolInvokeSource, [regex]::Escape('meta_mode = [string]$envelope.MetaMode')).Count -ne 1 -or
    [regex]::Matches($codexToolInvokeSource, [regex]::Escape('meta_present = [bool]$envelope.MetaPresent')).Count -ne 1 -or
    [regex]::Matches($codexToolInvokeSource, [regex]::Escape('meta_sha256 = $envelope.MetaSha256')).Count -ne 1 -or
    [regex]::Matches($codexToolInvokeSource, [regex]::Escape('meta_commitment_sha256 = $metaCommitmentSha256')).Count -ne 1 -or
    [regex]::Matches($callCountGuardSource, "'TASK051_CODEX_[A-Z0-9_]+_REJECTED'").Count -ne 6 -or
    [regex]::Matches($codexToolInvokeSource, [regex]::Escape($summaryWriterFragment)).Count -ne 1 -or
    [regex]::Matches($codexToolInvokeSource, [regex]::Escape('Resolve-Task051CodexToolFailure -Stage $failureStage')).Count -ne 1 -or
    [regex]::Matches($codexToolInvokeSource, [regex]::Escape("throw 'TASK038_CURRENT_CODEX_TOOL_CLEANUP_REJECTED'")).Count -ne 1 -or
    [regex]::Matches($codexToolInvokeSource, [regex]::Escape('Get-Task051CodexEventSummary -Events $events -Phase $Phase -ExpectedTool $Tool')).Count -ne 1 -or
    [regex]::Matches($codexToolInvokeSource, [regex]::Escape('Get-Task051ExecStructuredContent -Events $events -Phase $Phase -Tool $Tool')).Count -ne 1 -or
    [regex]::Matches($codexToolInvokeSource, [regex]::Escape("task051-' + `$Phase + '-codex-event-summary.json")).Count -ne 1 -or
    [regex]::Matches($codexToolInvokeSource, [regex]::Escape('Assert-Task051CodexPhaseTool -Phase $Phase -Tool $Tool')).Count -ne 1 -or
    [regex]::Matches($codexToolInvokeSource, 'Execution-only request\. Your first and only action').Count -ne 2 -or
    $codexToolInvokeSource.IndexOf('retry', [StringComparison]::OrdinalIgnoreCase) -ge 0 -or
    $codexToolInvokeSource.IndexOf('throw $message', [StringComparison]::Ordinal) -ge 0 -or
    $codexToolInvokeSource.IndexOf('throw $_', [StringComparison]::Ordinal) -ge 0
) {
    throw 'TASK051_CODEX_TOOL_FAILURE_CLASSIFIER_SHAPE_REJECTED'
}
foreach ($phaseCallFragment in @(
    "Invoke-Task051CodexDiscovery -Phase 'discovery'",
    "Invoke-Task051CodexTool -Phase 'submit' -Tool 'lattice_task_submit'",
    "Invoke-Task051CodexTool -Phase 'status-pre-restart' -Tool 'lattice_task_status'",
    "Invoke-Task051CodexTool -Phase 'status-post-restart' -Tool 'lattice_task_status'"
)) {
    if ([regex]::Matches($runnerSource, [regex]::Escape($phaseCallFragment)).Count -ne 1) {
        throw 'TASK051_CODEX_PHASE_TOOL_SHAPE_REJECTED'
    }
}

$semanticFieldStart = $runnerSource.IndexOf('function Get-Task051PublicStatusSemanticField', [StringComparison]::Ordinal)
$publicStatusAssertStart = $runnerSource.IndexOf('function Assert-Task051PublicStatus', $semanticFieldStart, [StringComparison]::Ordinal)
$sameStatusAssertStart = $runnerSource.IndexOf('function Assert-Task051SameStatus', $publicStatusAssertStart, [StringComparison]::Ordinal)
if ($semanticFieldStart -lt 0 -or $publicStatusAssertStart -le $semanticFieldStart -or $sameStatusAssertStart -le $publicStatusAssertStart) {
    throw 'TASK051_PUBLIC_STATUS_FIELD_DIAGNOSTIC_SHAPE_REJECTED'
}
$semanticFieldSource = $runnerSource.Substring($semanticFieldStart, $publicStatusAssertStart - $semanticFieldStart)
$publicStatusAssertSource = $runnerSource.Substring($publicStatusAssertStart, $sameStatusAssertStart - $publicStatusAssertStart)
$semanticFieldFragments = @(
    "if ([string]`$Value.schema_version -cne `$script:Task051PublicStatusSchema) { return 'SCHEMA' }",
    "if ([string]`$Value.status -cne 'COMPLETED') { return 'STATUS' }",
    "if ([string]`$Value.task_state -cne 'COMPLETED') { return 'TASK_STATE' }",
    "if ([string]`$Value.task_ref -cnotmatch '\A[0-9a-f]{64}\z') { return 'TASK_REF' }",
    "if ([string]`$Value.ledger_head_digest -cnotmatch '\A[0-9a-f]{64}\z') { return 'LEDGER_HEAD' }",
    "if ([string]`$Value.result_digest -cnotmatch '\A[0-9a-f]{64}\z') { return 'RESULT_DIGEST' }",
    "return 'NONE'"
)
$semanticFieldFragmentIndex = -1
foreach ($semanticFieldFragment in $semanticFieldFragments) {
    $nextSemanticFieldFragmentIndex = $semanticFieldSource.IndexOf($semanticFieldFragment, $semanticFieldFragmentIndex + 1, [StringComparison]::Ordinal)
    if ($nextSemanticFieldFragmentIndex -le $semanticFieldFragmentIndex) {
        throw 'TASK051_PUBLIC_STATUS_FIELD_DIAGNOSTIC_SHAPE_REJECTED'
    }
    $semanticFieldFragmentIndex = $nextSemanticFieldFragmentIndex
}
if (
    [regex]::Matches($publicStatusAssertSource, [regex]::Escape('Get-Task051PublicStatusSemanticField -Value $Value')).Count -ne 1 -or
    [regex]::Matches($publicStatusAssertSource, [regex]::Escape('[switch]$DetailedFailure')).Count -ne 1 -or
    [regex]::Matches($publicStatusAssertSource, [regex]::Escape('$Value.status -is [string]')).Count -ne 1 -or
    [regex]::Matches($publicStatusAssertSource, [regex]::Escape('switch -CaseSensitive ([string]$Value.status)')).Count -ne 1 -or
    $publicStatusAssertSource.IndexOf('$Value.schema_version', [StringComparison]::Ordinal) -ge 0 -or
    $publicStatusAssertSource.IndexOf('$Value.task_state', [StringComparison]::Ordinal) -ge 0 -or
    $publicStatusAssertSource.IndexOf('$Value.task_ref', [StringComparison]::Ordinal) -ge 0 -or
    $publicStatusAssertSource.IndexOf('$Value.ledger_head_digest', [StringComparison]::Ordinal) -ge 0 -or
    $publicStatusAssertSource.IndexOf('$Value.result_digest', [StringComparison]::Ordinal) -ge 0
) {
    throw 'TASK051_PUBLIC_STATUS_FIELD_DIAGNOSTIC_SHAPE_REJECTED'
}
foreach ($internalFieldLeaf in @('SCHEMA', 'STATUS_NOT_SUBMITTED', 'STATUS_RECONCILIATION_REQUIRED', 'STATUS_FAILED', 'STATUS_LOWER_COMPLETED', 'STATUS_OTHER', 'TASK_STATE', 'TASK_REF', 'LEDGER_HEAD', 'RESULT_DIGEST')) {
    if ([regex]::Matches($publicStatusAssertSource, [regex]::Escape("'TASK051_PUBLIC_STATUS_" + $internalFieldLeaf + "_REJECTED'")).Count -ne 1) {
        throw 'TASK051_PUBLIC_STATUS_FIELD_DIAGNOSTIC_SHAPE_REJECTED'
    }
}

$bundleStart = $runnerSource.IndexOf('function Get-Task051OfficialCodexBundlePolicy', [StringComparison]::Ordinal)
if ($bundleStart -lt 0) {
    throw 'TASK051_OFFICIAL_CODEX_BUNDLE_SHAPE_REJECTED'
}
$bundleEnd = $runnerSource.IndexOf('function Assert-Task051PublicStatus', $bundleStart, [StringComparison]::Ordinal)
if ($bundleEnd -le $bundleStart) { throw 'TASK051_OFFICIAL_CODEX_BUNDLE_SHAPE_REJECTED' }
$bundleSource = $runnerSource.Substring($bundleStart, $bundleEnd - $bundleStart)
$bundleRequiredFragments = @(
    'function Assert-Task051OfficialCodexBundle',
    'function Copy-Task051OfficialCodexBundle',
    'Assert-Task051NoReparseAncestor -Path $path -Boundary $bundleTarget',
    'Assert-Task051RegularFile -Path $path',
    '(Get-Task051Sha256 -Path $path) -cne',
    '$versionOutput.Count -ne 1',
    "'codex-cli 0.146.0'",
    'Set-Task051OwnerOnlyAcl -Path $destination -Directory $false',
    '(Get-Task051Sha256 -Path $destination) -cne',
    '-BundleTargetRoot $destinationTarget -Boundary $DestinationBoundary -ValidateVersion'
)
foreach ($fragment in $bundleRequiredFragments) {
    if ($bundleSource.IndexOf($fragment, [StringComparison]::Ordinal) -lt 0) {
        throw ('TASK051_OFFICIAL_CODEX_BUNDLE_SHAPE_REJECTED|' + $fragment)
    }
}
if (
    [regex]::Matches($bundleSource, "RelativePath = 'codex-official\\").Count -ne 7 -or
    [regex]::Matches($bundleSource, [regex]::Escape('[void](Assert-Task051OfficialCodexBundle -BundleTargetRoot $SourceTargetRoot -Boundary $SourceBoundary)')).Count -ne 2
) {
    throw 'TASK051_OFFICIAL_CODEX_BUNDLE_SHAPE_REJECTED'
}
$mainStart = $runnerSource.IndexOf('$externalResourcesMayExist = $false', [StringComparison]::Ordinal)
if ($mainStart -lt 0) { throw 'TASK051_OFFICIAL_CODEX_BUNDLE_CLEANUP_SHAPE_REJECTED' }
$mainEnd = $runnerSource.IndexOf('if ((Get-Task051Sha256 -Path $originalConfig)', $mainStart, [StringComparison]::Ordinal)
if ($mainEnd -le $mainStart) { throw 'TASK051_OFFICIAL_CODEX_BUNDLE_CLEANUP_SHAPE_REJECTED' }
$mainSource = $runnerSource.Substring($mainStart, $mainEnd - $mainStart)
$mainPreludeStart = $runnerSource.IndexOf('if ($LibraryOnly) { return }', [StringComparison]::Ordinal)
if ($mainPreludeStart -lt 0 -or $mainPreludeStart -ge $mainStart) { throw 'TASK051_RUN_ROOT_PRETRY_SHAPE_REJECTED' }
$mainPreludeSource = $runnerSource.Substring($mainPreludeStart, $mainStart - $mainPreludeStart)
$baselineBeforeAllocation = $mainPreludeSource.IndexOf('$postgresProcessBaseline = @(Get-Task051PostgresProcessSnapshot)', [StringComparison]::Ordinal)
$runAllocationAfterBaseline = $mainPreludeSource.IndexOf('$runAllocation = New-Task051RunRoot -AllowedRoot $allowedRoot -RunId $runId', [StringComparison]::Ordinal)
if (
    $baselineBeforeAllocation -lt 0 -or
    $runAllocationAfterBaseline -le $baselineBeforeAllocation -or
    [regex]::Matches($mainPreludeSource, [regex]::Escape('$postgresProcessBaseline = @(Get-Task051PostgresProcessSnapshot)')).Count -ne 1 -or
    [regex]::Matches($mainPreludeSource, [regex]::Escape('Assert-Task051NoReparseAncestor -Path $runRoot')).Count -ne 0
) {
    throw 'TASK051_RUN_ROOT_PRETRY_SHAPE_REJECTED'
}
$resourcesArmed = $mainSource.IndexOf('$externalResourcesMayExist = $true', [StringComparison]::Ordinal)
$harnessInvocation = $mainSource.IndexOf('$harnessOutput = @(& $generatedTask019', [StringComparison]::Ordinal)
$aliasReleaseSafe = $mainSource.IndexOf('$externalResourcesMayExist -and', [StringComparison]::Ordinal)
$privateBundleCleanup = $mainSource.IndexOf("Remove-Task051OwnedDirectory -Path `$privateOfficialCodexBundleTarget -AllowedRoot `$runRoot -FailureCode 'TASK051_OFFICIAL_CODEX_BUNDLE_CLEANUP_REJECTED'", [StringComparison]::Ordinal)
$aliasRemoval = if ($privateBundleCleanup -ge 0) {
    $mainSource.IndexOf('Remove-Task051RunRootAlias -Alias $runAlias', $privateBundleCleanup, [StringComparison]::Ordinal)
}
else { -1 }
if (
    [regex]::Matches($mainSource, [regex]::Escape('$externalResourcesMayExist')).Count -ne 3 -or
    $resourcesArmed -lt 0 -or
    $harnessInvocation -le $resourcesArmed -or
    $aliasReleaseSafe -lt 0 -or
    $aliasReleaseSafe -le $harnessInvocation -or
    $privateBundleCleanup -le $aliasReleaseSafe -or
    $aliasRemoval -le $privateBundleCleanup -or
    $runnerSource.IndexOf("[IO.Directory]::Delete(('\\?\' + `$fullPath), `$true)", [StringComparison]::Ordinal) -lt 0 -or
    $runnerSource.IndexOf('if (Test-Path -LiteralPath $fullPath)', [StringComparison]::Ordinal) -lt 0
) {
    throw 'TASK051_OFFICIAL_CODEX_BUNDLE_CLEANUP_SHAPE_REJECTED'
}

$runRootFunctionStart = $runnerSource.IndexOf('function Get-Task051RunSlot {', [StringComparison]::Ordinal)
$runRootFunctionEnd = $runnerSource.IndexOf('function Assert-Task051PublicStatus {', $runRootFunctionStart, [StringComparison]::Ordinal)
$atomicDirectoryFunctionStart = $runnerSource.IndexOf('function Assert-Task051AtomicDirectoryStage {', [StringComparison]::Ordinal)
$atomicDirectoryFunctionEnd = $runnerSource.IndexOf('function Initialize-Task051CargoHome {', $atomicDirectoryFunctionStart, [StringComparison]::Ordinal)
$task038TransformStart = $runnerSource.IndexOf('function Convert-Task051Task038Source {', [StringComparison]::Ordinal)
$task038TransformEnd = $runnerSource.IndexOf('function Convert-Task051Task019Source {', $task038TransformStart, [StringComparison]::Ordinal)
if (
    $runRootFunctionStart -lt 0 -or
    $runRootFunctionEnd -le $runRootFunctionStart -or
    $atomicDirectoryFunctionStart -lt 0 -or
    $atomicDirectoryFunctionEnd -le $atomicDirectoryFunctionStart -or
    $task038TransformStart -lt 0 -or
    $task038TransformEnd -le $task038TransformStart
) {
    throw 'TASK051_COMPACT_DELIVERY_SHAPE_REJECTED'
}
$runRootFunctionSource = $runnerSource.Substring($runRootFunctionStart, $runRootFunctionEnd - $runRootFunctionStart)
$atomicDirectoryFunctionSource = $runnerSource.Substring($atomicDirectoryFunctionStart, $atomicDirectoryFunctionEnd - $atomicDirectoryFunctionStart)
$task038TransformSource = $runnerSource.Substring($task038TransformStart, $task038TransformEnd - $task038TransformStart)
if (
    [regex]::Matches($runRootFunctionSource, [regex]::Escape(".Substring(0, 6)")).Count -ne 1 -or
    [regex]::Matches($runRootFunctionSource, [regex]::Escape("schema_version = 'lattice.task051.run-root.v1'")).Count -ne 1 -or
    [regex]::Matches($runRootFunctionSource, [regex]::Escape('for ($attempt = 0; $attempt -lt 64; $attempt++)')).Count -ne 1 -or
    [regex]::Matches($runRootFunctionSource, [regex]::Escape("`$stageName = '.task051-stage-' + `$RunId + '-' + ('{0:d2}' -f `$attempt) + '-' + [Guid]::NewGuid().ToString('N')")).Count -ne 1 -or
    [regex]::Matches($runRootFunctionSource, [regex]::Escape('New-Task051OwnerOnlyDirectory -Path $stage')).Count -ne 1 -or
    [regex]::Matches($runRootFunctionSource, [regex]::Escape('New-Task051OwnerOnlyDirectory -Path $candidate')).Count -ne 0 -or
    [regex]::Matches($runRootFunctionSource, [regex]::Escape('[IO.Directory]::Move($stage, $candidate)')).Count -ne 1 -or
    [regex]::Matches($runRootFunctionSource, [regex]::Escape('$candidateCommitted = $true')).Count -ne 1 -or
    [regex]::Matches($runRootFunctionSource, [regex]::Escape('if ($candidateCommitted -and (Test-Path -LiteralPath $candidate -PathType Container))')).Count -ne 1 -or
    [regex]::Matches($runRootFunctionSource, [regex]::Escape('[string]::Equals($markerText, $canonicalMarkerText, [StringComparison]::Ordinal')).Count -ne 1 -or
    [regex]::Matches($runRootFunctionSource, [regex]::Escape("MarkerPath = Join-Path `$candidate '.task051-run-root.json'")).Count -ne 1 -or
    [regex]::Matches($atomicDirectoryFunctionSource, [regex]::Escape('[IO.Directory]::Move($stage, $destination)')).Count -ne 1 -or
    [regex]::Matches($atomicDirectoryFunctionSource, [regex]::Escape("'.a-' + [Guid]::NewGuid().ToString('N')")).Count -ne 1 -or
    [regex]::Matches($atomicDirectoryFunctionSource, [regex]::Escape("`$markerPath = Join-Path `$fullDirectory '.a'")).Count -ne 1 -or
    [regex]::Matches($atomicDirectoryFunctionSource, [regex]::Escape("`$markerPath = Join-Path `$stage '.a'")).Count -ne 1 -or
    [regex]::Matches($atomicDirectoryFunctionSource, [regex]::Escape("[IO.Directory]::Delete(('\\?\' + `$destination)")).Count -ne 0 -or
    [regex]::Matches($atomicDirectoryFunctionSource, [regex]::Escape('Assert-Task051AtomicDirectoryStage -ParentPath $fullParent -DirectoryPath $stage')).Count -ne 2 -or
    [regex]::Matches($runnerSource, [regex]::Escape('$compactMarkerMutationBytes.Add($compactUtf8.GetBytes($compactCanonicalMarkerText.Replace(')).Count -ne 3 -or
    [regex]::Matches($task038TransformSource, [regex]::Escape("Join-Path `$repositoryTarget 'x'")).Count -ne 1 -or
    [regex]::Matches($task038TransformSource, [regex]::Escape('New-Task051OwnerOnlyDirectory -Path $deliveryRootCandidate')).Count -ne 0 -or
    [regex]::Matches($task038TransformSource, [regex]::Escape('New-Task051AtomicOwnerOnlyEmptyDirectory')).Count -ne 1 -or
    [regex]::Matches($task038TransformSource, [regex]::Escape("-MarkerSchema 'lattice.task051.delivery-stage.v1'")).Count -ne 1 -or
    [regex]::Matches($task038TransformSource, [regex]::Escape("-CleanupFailureCode 'TASK038_DELIVERY_ROOT_CLEANUP_REJECTED'")).Count -ne 1 -or
    [regex]::Matches($task038TransformSource, [regex]::Escape("Join-Path `$deliveryRoot ('task-' + [string]`$submitted.task_ref)")).Count -ne 1 -or
    [regex]::Matches($task038TransformSource, [regex]::Escape("Join-Path `$taskDeliveryRoot 'repo'")).Count -ne 1 -or
    [regex]::Matches($task038TransformSource, [regex]::Escape("Join-Path `$deliveryRoot 'repo'")).Count -ne 0 -or
    [regex]::Matches($runnerSource, [regex]::Escape('$runAllocation = New-Task051RunRoot -AllowedRoot $allowedRoot -RunId $runId')).Count -ne 1 -or
    [regex]::Matches($runnerSource, [regex]::Escape("schema_version = 'lattice.task051.p0-platform-live-acceptance.v2'")).Count -ne 1 -or
    [regex]::Matches($runnerSource, [regex]::Escape('$runRoot = [IO.Path]::GetFullPath((Join-Path $allowedRoot $runId))')).Count -ne 0
) {
    throw 'TASK051_COMPACT_DELIVERY_SHAPE_REJECTED'
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
    'git reset --hard',
    'GetExitCodeProcess',
    'task038-official-codex\0.146.0\codex.exe',
    "`$deliveryRoot = Join-Path `$fixtureRoot 'delivery'"
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
    'TASK051_PROCESS_OPEN_CLASSIFIER_SELF_TEST=PASS',
    'TASK051_RETAINED_PROCESS_AUTHORITY_SELF_TEST=PASS',
    'TASK051_PROCESS_LIFETIME_SELF_TEST=PASS',
    'TASK051_MCP_SERVER_NATURAL_EXIT_SELF_TEST=PASS',
    'TASK051_MCP_SESSION_OPEN_PARSE_DIAGNOSTIC_SELF_TEST=PASS',
    'TASK051_CODEX_TOOL_FAILURE_CLASSIFIER_SELF_TEST=PASS',
    'TASK051_CODEX_CALL_COUNT_PHASE_SELF_TEST=PASS',
    'TASK051_CODEX_EVENT_SUMMARY_SELF_TEST=PASS',
    'TASK051_CODEX_TOOL_FIELD_DIAGNOSTIC_SELF_TEST=PASS',
    'TASK051_CODEX_TOOL_RESULT_META_EVIDENCE_SELF_TEST=PASS',
    'TASK051_CODEX_PHASE_TOOL_NO_MATERIALIZATION_SELF_TEST=PASS',
    'TASK051_ATOMIC_DIRECTORY_SELF_TEST=PASS',
    'TASK051_CODEX_PER_TOOL_APPROVAL_SELF_TEST=PASS',
    'TASK051_OFFICIAL_CODEX_BUNDLE_SELF_TEST=PASS',
    'TASK051_COMPACT_RUN_ROOT_SELF_TEST=PASS',
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
