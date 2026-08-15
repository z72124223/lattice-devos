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
    'Get-Task051OwnedProcessEvidence',
    'IsProcessInJob',
    'QueryFullProcessImageName',
    'TASK038_CURRENT_CODEX_DISCOVERY_SESSION_OPEN_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_JOB_MEMBERSHIP_REJECTED',
    'TASK038_CURRENT_CODEX_DISCOVERY_PROCESS_IMAGE_REJECTED',
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

$discoveryStart = $runnerSource.IndexOf('function Invoke-Task051CodexDiscovery', [StringComparison]::Ordinal)
$discoveryEnd = $runnerSource.IndexOf('function Get-Task051ExecStructuredContent', $discoveryStart, [StringComparison]::Ordinal)
if ($discoveryStart -lt 0 -or $discoveryEnd -le $discoveryStart) {
    throw 'TASK051_APP_SERVER_PROCESS_AUTHORITY_SHAPE_REJECTED'
}
$discoverySource = $runnerSource.Substring($discoveryStart, $discoveryEnd - $discoveryStart)
$sessionOpenRead = $discoverySource.IndexOf('$sessionOpen = Read-Task051McpSessionOpen', [StringComparison]::Ordinal)
$serverPidCapture = $discoverySource.IndexOf('$serverProcessId = [int]$sessionOpen.ProcessId', [StringComparison]::Ordinal)
$processAuthority = $discoverySource.IndexOf('$processEvidence = Get-Task051OwnedProcessEvidence', [StringComparison]::Ordinal)
if (
    $sessionOpenRead -lt 0 -or
    $serverPidCapture -le $sessionOpenRead -or
    $processAuthority -le $serverPidCapture
) {
    throw 'TASK051_APP_SERVER_PROCESS_AUTHORITY_SHAPE_REJECTED'
}
foreach ($forbiddenDiscoveryFragment in @('ParentProcessId =', '.ExecutablePath')) {
    if ($discoverySource.IndexOf($forbiddenDiscoveryFragment, [StringComparison]::Ordinal) -ge 0) {
        throw ('TASK051_APP_SERVER_PROCESS_AUTHORITY_SHAPE_REJECTED|' + $forbiddenDiscoveryFragment)
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
    'task038-official-codex\0.146.0\codex.exe'
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
    'TASK051_OFFICIAL_CODEX_BUNDLE_SELF_TEST=PASS',
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
