[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:ExactMarker = 'TASK038_FOUR_TOOL_ACCEPTANCE=PASS'
$script:RepoRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$script:Runner = Join-Path $PSScriptRoot 'run-task038-four-tool-acceptance.ps1'
$script:Utf8 = [Text.UTF8Encoding]::new($false)
$script:CleanSeedCommit = '2b424ec9a5401a6fbdc4f37d3d401592331afca0'
$script:CleanSeedTree = '9c4cad5b4b3e3362521643b6dd283d31cde29345'
$script:P007Commit = 'db56d471a1eec2dece06523661e3b571d345cbb2'
$script:P007Tree = '4bd6102f6a83b5984bcc993b74a090a33dcbcea9'
$script:P005Commit = '236b1d6f8362da19d298d65fd652e045cd413a02'
$script:P005Tree = '3d2a01383be7b27871e86fe2df42fe0fc8728be9'
$script:ExpectedToolErrorContractSha256 = 'f9d506179a8d6528c1a5291b704b3f7c4bfbe1bfa447027619a6ea0aaed7dc71'
$script:BridgeRunner = Join-Path $PSScriptRoot 'run-task038-task-submit.ps1'
$script:NativeIdentityHelper = Join-Path $PSScriptRoot 'windows-native-path-identity.ps1'
$script:RuntimeComposition = Join-Path $script:RepoRoot 'apps\lattice-runtime\src\composition.rs'
$script:McpSource = Join-Path $script:RepoRoot 'apps\lattice-runtime\src\mcp.rs'

function Get-GateClassification {
    param(
        [Parameter(Mandatory = $true)][int]$ExitCode,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Lines
    )

    $passCount = @($Lines | Where-Object { $_ -ceq $script:ExactMarker }).Count
    $skipCount = @($Lines | Where-Object { $_ -match '(?i)(?:^|[=:\s])SKIP(?:$|[=:\s])' }).Count
    if ($ExitCode -eq 0 -and $passCount -eq 1 -and $skipCount -eq 0) { return 'PASS' }
    if ($ExitCode -eq 0 -and $passCount -eq 0 -and $skipCount -gt 0) { return 'NOT_RUN' }
    return 'FAIL'
}

function Invoke-Runner {
    param(
        [Parameter(Mandatory = $true)][string]$Binary,
        [Parameter(Mandatory = $true)][string]$BinarySha,
        [Parameter(Mandatory = $true)][string]$Commit,
        [Parameter(Mandatory = $true)][string]$Mode,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot,
        [switch]$IncludeCallerBinary,
        [hashtable]$AdditionalArguments = @{}
    )

    $arguments = @(
        '-NoLogo', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $script:Runner,
        '-ExpectedSourceCommit', $Commit,
        '-SourceRepository', $script:RepoRoot,
        '-Mode', $Mode,
        '-SessionTimeoutSeconds', '30',
        '-EvidenceRoot', $EvidenceRoot
    )
    if ($Mode -cne 'FULL' -or $IncludeCallerBinary) {
        $arguments += @('-LatticedExecutable', $Binary, '-ExpectedBinarySha256', $BinarySha)
    }
    if ($Mode -ceq 'PROTOCOL_ONLY' -and -not [string]::IsNullOrWhiteSpace($script:HarnessObservedCounterPath)) {
        $arguments += @('-HarnessObservedCounterPath', $script:HarnessObservedCounterPath)
    }
    foreach ($name in @($AdditionalArguments.Keys | Sort-Object)) {
        $arguments += '-' + $name
        if ($AdditionalArguments[$name] -isnot [Management.Automation.SwitchParameter]) {
            $arguments += [string]$AdditionalArguments[$name]
        }
    }
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& (Join-Path $PSHOME 'powershell.exe') @arguments 2>&1 | ForEach-Object { [string]$_ })
        $exitCode = [int]$LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previous
    }
    return [pscustomobject]@{
        ExitCode = $exitCode
        Lines = $output
        Classification = Get-GateClassification -ExitCode $exitCode -Lines $output
    }
}

function Get-FinalEvidence {
    param([Parameter(Mandatory = $true)][string]$EvidenceRoot)

    return Get-Content -LiteralPath (Join-Path $EvidenceRoot 'final.json') -Raw -Encoding UTF8 | ConvertFrom-Json
}

function Assert-FixedFailure {
    param(
        [Parameter(Mandatory = $true)]$Run,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    if ($Run.Classification -cne 'FAIL' -or @($Run.Lines | Where-Object { $_ -ceq $script:ExactMarker }).Count -ne 0) {
        throw 'TASK038_ACCEPT_TEST_NEGATIVE_MARKER_REJECTED'
    }
    $final = Get-FinalEvidence -EvidenceRoot $EvidenceRoot
    if ([string]$final.status -cne 'FAIL' -or [string]$final.failure_code -cne $FailureCode) {
        throw ('TASK038_ACCEPT_TEST_FIXED_FAILURE_REJECTED_' + $FailureCode)
    }
}

if (-not (Test-Path -LiteralPath $script:Runner -PathType Leaf)) {
    throw 'TASK038_ACCEPT_TEST_RUNNER_MISSING'
}
foreach ($dependency in @($script:BridgeRunner, $script:NativeIdentityHelper, $script:RuntimeComposition, $script:McpSource)) {
    if (-not (Test-Path -LiteralPath $dependency -PathType Leaf)) {
        throw 'TASK038_ACCEPT_TEST_SHARED_DEPENDENCY_MISSING'
    }
}
$p007Resolved = (& git -C $script:RepoRoot rev-parse ($script:P007Commit + '^{commit}')).Trim().ToLowerInvariant()
$p007Tree = (& git -C $script:RepoRoot rev-parse ($script:P007Commit + '^{tree}')).Trim().ToLowerInvariant()
$null = & git -C $script:RepoRoot merge-base --is-ancestor $script:P007Commit HEAD
if ($p007Resolved -cne $script:P007Commit -or $p007Tree -cne $script:P007Tree -or $LASTEXITCODE -ne 0) {
    throw 'TASK038_ACCEPT_TEST_P007_IDENTITY_NOT_MATERIALIZED'
}
$p005Resolved = (& git -C $script:RepoRoot rev-parse ($script:P005Commit + '^{commit}')).Trim().ToLowerInvariant()
$p005Tree = (& git -C $script:RepoRoot rev-parse ($script:P005Commit + '^{tree}')).Trim().ToLowerInvariant()
$null = & git -C $script:RepoRoot merge-base --is-ancestor $script:P005Commit HEAD
if ($p005Resolved -cne $script:P005Commit -or $p005Tree -cne $script:P005Tree -or $LASTEXITCODE -ne 0) {
    throw 'TASK038_ACCEPT_TEST_P005_BRIDGE_NOT_MATERIALIZED'
}
foreach ($scriptPath in @($script:BridgeRunner, $script:NativeIdentityHelper)) {
    $dependencyTokens = $null
    $dependencyErrors = $null
    $null = [Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$dependencyTokens, [ref]$dependencyErrors)
    if (@($dependencyErrors).Count -ne 0) {
        throw 'TASK038_ACCEPT_TEST_SHARED_DEPENDENCY_AST_REJECTED'
    }
}
$bridgeText = Get-Content -LiteralPath $script:BridgeRunner -Raw -Encoding UTF8
$identityText = Get-Content -LiteralPath $script:NativeIdentityHelper -Raw -Encoding UTF8
$compositionText = Get-Content -LiteralPath $script:RuntimeComposition -Raw -Encoding UTF8
$mcpText = Get-Content -LiteralPath $script:McpSource -Raw -Encoding UTF8
$runnerText = Get-Content -LiteralPath $script:Runner -Raw -Encoding UTF8
foreach ($required in @('windows-native-path-identity.ps1','LATTICE_TASK_INGRESS_KIND','LATTICE_TASK_INGRESS_PROFILE_SHA256','LOCAL_CANONICAL_MCP_ACCEPTANCE','lattice_task_submit','lattice_task_status')) {
    if (-not $bridgeText.Contains($required)) { throw 'TASK038_ACCEPT_TEST_P005_BRIDGE_NOT_MATERIALIZED' }
}
foreach ($required in @('GetFileInformationByHandleEx','New-LatticeWindowsNativeContainmentSnapshot','Assert-LatticeWindowsNativeContainmentSnapshot')) {
    if (-not $identityText.Contains($required)) { throw 'TASK038_ACCEPT_TEST_P005_NATIVE_IDENTITY_NOT_MATERIALIZED' }
}
foreach ($required in @('OFFICIAL_BUNDLE_FILE_ROLES','GetFileInformationByHandle','LATTICE_OFFICIAL_CODEX_IDENTITY_REJECTED','LATTICE_TASK_INGRESS_PROFILE_SHA256')) {
    if (-not $compositionText.Contains($required)) { throw 'TASK038_ACCEPT_TEST_P007_RUNTIME_IDENTITY_NOT_MATERIALIZED' }
}
foreach ($required in @('lattice_delivery_run','lattice_delivery_status','lattice_task_submit','lattice_task_status')) {
    if (-not $mcpText.Contains($required)) { throw 'TASK038_ACCEPT_TEST_FOUR_TOOL_SOURCE_NOT_MATERIALIZED' }
}
foreach ($required in @(
    '236b1d6f8362da19d298d65fd652e045cd413a02',
    'lattice.mcp.acceptance-dispatch.v1',
    'lattice.task038.mcp-acceptance-dispatch-evidence.v1',
    'TASK038_ACCEPT_PRODUCTION_NEGATIVE_EFFECT_AUTHORITY_NOT_AVAILABLE',
    'lattice.task038.production-session-effect-observation.v1',
    'lattice.task038.database-binding.v1',
    'LATTICE_TASK038_P006_CURRENT_CANDIDATE_ACCEPTANCE_V1',
    '882a5a073a88817f6c6d4c8827df1e4269ff226d52cf6f47c9883e91088c6345',
    'e43adb9c5032e7efc63eebb44c5d32b142b34e5f4207666fed2dc7a51d43b630',
    'abe89b0767a8cd0f956059aa5a5a93cd1042efc6194d000c2501da3e23babbd2'
)) {
    if (-not $runnerText.Contains($required)) { throw 'TASK038_ACCEPT_TEST_PRODUCTION_CONTRACT_NOT_MATERIALIZED' }
}
if ($runnerText.Contains('LATTICE_TASK038_P006_REMEDIATION_REVIEW_V1')) {
    throw 'TASK038_ACCEPT_TEST_SELF_AUTHORED_REVIEW_NOT_REMOVED'
}
foreach ($required in @('independent_review_evaluation', 'NOT_EVALUATED', 'NON_LIVE_PARSER_ONLY', 'TASK038_ACCEPT_TUNNEL_LIFECYCLE_AUTHORITY_NOT_AVAILABLE')) {
    if (-not $runnerText.Contains($required)) { throw 'TASK038_ACCEPT_TEST_EXTERNAL_REVIEW_LIFECYCLE_GATE_MISSING' }
}
if ($runnerText.Contains('authoritative_observation = $true')) {
    throw 'TASK038_ACCEPT_TEST_SYNTHETIC_AUTHORITATIVE_ZERO_NOT_REMOVED'
}
if (-not $runnerText.Contains('TASK038_ACCEPT_PRODUCTION_NEGATIVE_EFFECT_AUTHORITY_NOT_AVAILABLE')) {
    throw 'TASK038_ACCEPT_TEST_PRODUCTION_NEGATIVE_EFFECT_GATE_MISSING'
}
if ($runnerText.Contains('fresh_marker_checked = $true')) {
    throw 'TASK038_ACCEPT_TEST_REWRITABLE_MARKER_FRESHNESS_CLAIM_NOT_REMOVED'
}
foreach ($required in @('Assert-PostgresRestartChronology', 'Assert-PostgresHolderReceiptAuthority', 'TASK038_ACCEPT_POSTGRES_HOLDER_RECEIPT_NOT_AVAILABLE')) {
    if (-not $runnerText.Contains($required)) { throw 'TASK038_ACCEPT_TEST_POSTGRES_HOLDER_AUTHORITY_GATE_MISSING' }
}
if ((Get-GateClassification -ExitCode 0 -Lines @('SKIP: live prerequisite unavailable')) -cne 'NOT_RUN') {
    throw 'TASK038_ACCEPT_TEST_SKIP_NOT_RUN_REJECTED'
}
if ((Get-GateClassification -ExitCode 0 -Lines @($script:ExactMarker, $script:ExactMarker)) -cne 'FAIL') {
    throw 'TASK038_ACCEPT_TEST_DUPLICATE_MARKER_REJECTED'
}
if ((Get-GateClassification -ExitCode 0 -Lines @('ordinary output')) -cne 'FAIL') {
    throw 'TASK038_ACCEPT_TEST_MISSING_MARKER_REJECTED'
}
if ((Get-GateClassification -ExitCode 0 -Lines @($script:ExactMarker)) -cne 'PASS') {
    throw 'TASK038_ACCEPT_TEST_EXACT_MARKER_REJECTED'
}

# Exercise the production database-name function without executing the runner body.
$tokens = $null
$parseErrors = $null
$runnerAst = [Management.Automation.Language.Parser]::ParseFile($script:Runner, [ref]$tokens, [ref]$parseErrors)
if (@($parseErrors).Count -ne 0) { throw 'TASK038_ACCEPT_TEST_RUNNER_AST_REJECTED' }
$databaseNameFunction = $runnerAst.Find({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq 'Get-Task019ProductionDatabaseName'
}, $true)
if ($null -eq $databaseNameFunction) { throw 'TASK038_ACCEPT_TEST_DATABASE_NAME_FUNCTION_MISSING' }

$scriptRunIdParameter = @($runnerAst.ParamBlock.Parameters | Where-Object { $_.Name.VariablePath.UserPath -ceq 'PostgresRunId' })
$functionRunIdParameter = @($databaseNameFunction.Body.ParamBlock.Parameters | Where-Object { $_.Name.VariablePath.UserPath -ceq 'RunId' })
if ($scriptRunIdParameter.Count -ne 1 -or $functionRunIdParameter.Count -ne 1) {
    throw 'TASK038_ACCEPT_TEST_DATABASE_RUN_ID_PARAMETER_REJECTED'
}
$expectedRunIdValidator = '[ValidateScript({$_-cmatch''\A[0-9a-f]{32}\z''})]'
foreach ($parameter in @($scriptRunIdParameter[0], $functionRunIdParameter[0])) {
    $validators = @($parameter.Attributes | Where-Object { $_.TypeName.Name -ceq 'ValidateScript' })
    if (
        $validators.Count -ne 1 -or
        ([string]$validators[0].Extent.Text -replace '\s+', '') -cne $expectedRunIdValidator
    ) {
        throw 'TASK038_ACCEPT_TEST_DATABASE_RUN_ID_VALIDATOR_REJECTED'
    }
}

. ([scriptblock]::Create($databaseNameFunction.Extent.Text))
$validRunId = '0123456789abcdef0123456789abcdef'
if ((Get-Task019ProductionDatabaseName -RunId $validRunId) -cne 'lattice_task019_01234567_base') {
    throw 'TASK038_ACCEPT_TEST_DATABASE_NAME_REJECTED'
}
$invalidRunIds = [ordered]@{
    uppercase = '0123456789ABCDEF0123456789ABCDEF'
    short = '0123456789abcdef0123456789abcde'
    nonhex = '0123456789abcdef0123456789abcdeg'
    leading_whitespace = ' 0123456789abcdef0123456789abcdef'
    trailing_whitespace = '0123456789abcdef0123456789abcdef '
    trailing_newline = "0123456789abcdef0123456789abcdef`n"
}
foreach ($caseName in $invalidRunIds.Keys) {
    $invalidRunId = [string]$invalidRunIds[$caseName]
    $rejected = $false
    try { $null = Get-Task019ProductionDatabaseName -RunId $invalidRunId }
    catch { $rejected = $true }
    if (-not $rejected) { throw ('TASK038_ACCEPT_TEST_DATABASE_RUN_ID_VALIDATION_REJECTED_' + $caseName) }
}

$testRoot = Join-Path $script:RepoRoot ('target\task038-four-tool-acceptance-test\' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($testRoot) | Out-Null

function Import-RunnerFunction {
    param([Parameter(Mandatory = $true)][string]$Name)

    $functionAst = $runnerAst.Find({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq $Name
    }, $true)
    if ($null -eq $functionAst) { throw ('TASK038_ACCEPT_TEST_RUNNER_FUNCTION_MISSING_' + $Name) }
    return [scriptblock]::Create($functionAst.Extent.Text)
}

foreach ($name in @(
    'Get-CanonicalPath', 'Get-StringSha256', 'Get-FileSha256', 'Assert-ExactJsonKeys',
    'Test-JsonInteger', 'Initialize-FileIdentityInterop', 'Get-NativeFileIdentity',
    'Get-NativeDirectoryIdentity', 'Get-AuthoritativeNativeFileIdentity', 'Get-LifecycleProcessIdentityParts',
    'Get-ApprovedRustToolchainReceipt', 'Set-ClosedRustBuildEnvironment', 'Assert-CandidateBuildOutputAbsent',
    'Read-McpAcceptanceEvidence', 'Read-TunnelLifecycleReceipt', 'Assert-LiveTunnelLifecycleAuthority', 'Get-DirectoryFootprint',
    'ConvertTo-CanonicalJson', 'Get-DomainSeparatedCommitment', 'New-ZeroProductionSessionEffectReceipt',
    'Assert-ProductionNegativeEffectAuthority',
    'Invoke-RepositoryGitText', 'Get-DeliveryGitEffectReceipt', 'New-SubmitProductionSessionEffectReceipt',
    'Assert-PostgresPortPolicy', 'Assert-PostgresRestartChronology', 'Assert-PostgresHolderReceiptAuthority', 'Get-PostgresDatabaseBinding'
)) { . (Import-RunnerFunction -Name $name) }
. $script:NativeIdentityHelper
$script:P005LifecycleCommit = '392c39cbbc8d416b5d89b872b0be119336946247'
$script:ExpectedTools = @('lattice_delivery_run', 'lattice_delivery_status', 'lattice_task_status', 'lattice_task_submit')
$script:ApprovedRustToolchain = 'stable-x86_64-pc-windows-msvc'
$script:ApprovedCargoVersion = 'cargo 1.97.1 (c980f4866 2026-06-30)'
$script:ApprovedRustcVersion = 'rustc 1.97.1 (8bab26f4f 2026-07-14)'
$script:ApprovedCargoShimSha256 = '86478e53f769379d7f0ebfa7c9aa97cb76ca92233f79aa2cc0dbee2efaac73c7'
$script:ApprovedCargoSha256 = 'ddfbad20b31b918d3439d070945ec59bbfe037a6ec0ab5b584459e69c8b37d1b'
$script:ApprovedRustcSha256 = 'cf79cfd77b0a144c56a0a6af6bf10bcdf095a73718cd4bf2b9d4fe2d2cbded55'
$script:ApprovedCargoShimNativeIdentity = 'lattice.win-file-id.v1:fcc833adc8336554:000000000000000000760000000647ed:f'
$script:ApprovedCargoNativeIdentity = 'lattice.win-file-id.v1:fcc833adc8336554:00000000000000000017000000092ef3:f'
$script:ApprovedRustcNativeIdentity = 'lattice.win-file-id.v1:fcc833adc8336554:00000000000000000001000000175377:f'

$approvedToolchain = Get-ApprovedRustToolchainReceipt
if (
    [string]$approvedToolchain.schema_version -cne 'lattice.task038.approved-rust-toolchain.v1' -or
    [string]$approvedToolchain.toolchain -cne 'stable-x86_64-pc-windows-msvc' -or
    [string]$approvedToolchain.cargo_sha256 -cne 'ddfbad20b31b918d3439d070945ec59bbfe037a6ec0ab5b584459e69c8b37d1b' -or
    [string]$approvedToolchain.rustc_sha256 -cne 'cf79cfd77b0a144c56a0a6af6bf10bcdf095a73718cd4bf2b9d4fe2d2cbded55' -or
    [string]$approvedToolchain.cargo_version -cne 'cargo 1.97.1 (c980f4866 2026-06-30)' -or
    [string]$approvedToolchain.rustc_version -cne 'rustc 1.97.1 (8bab26f4f 2026-07-14)'
) { throw 'TASK038_ACCEPT_TEST_APPROVED_TOOLCHAIN_RECEIPT_REJECTED' }
$closedBuildStartInfo = [Diagnostics.ProcessStartInfo]::new()
$closedBuildStartInfo.UseShellExecute = $false
$closedBuildStartInfo.EnvironmentVariables['RUSTC_WRAPPER'] = 'malicious-wrapper.exe'
$closedBuildStartInfo.EnvironmentVariables['RUSTC_WORKSPACE_WRAPPER'] = 'malicious-workspace-wrapper.exe'
$closedBuildStartInfo.EnvironmentVariables['RUSTFLAGS'] = '-C linker=malicious-linker.exe'
$closedBuildStartInfo.EnvironmentVariables['CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER'] = 'malicious-linker.exe'
Set-ClosedRustBuildEnvironment -StartInfo $closedBuildStartInfo -Toolchain $approvedToolchain -SourceDateEpoch '1786400000'
if (
    $closedBuildStartInfo.EnvironmentVariables.ContainsKey('RUSTC_WRAPPER') -or
    $closedBuildStartInfo.EnvironmentVariables.ContainsKey('RUSTC_WORKSPACE_WRAPPER') -or
    $closedBuildStartInfo.EnvironmentVariables.ContainsKey('RUSTFLAGS') -or
    $closedBuildStartInfo.EnvironmentVariables.ContainsKey('CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER') -or
    [string]$closedBuildStartInfo.EnvironmentVariables['RUSTC'] -cne [string]$approvedToolchain.rustc_path -or
    [string]$closedBuildStartInfo.EnvironmentVariables['RUSTUP_TOOLCHAIN'] -cne [string]$approvedToolchain.toolchain -or
    [string]$closedBuildStartInfo.EnvironmentVariables['SOURCE_DATE_EPOCH'] -cne '1786400000'
) { throw 'TASK038_ACCEPT_TEST_CLOSED_BUILD_ENVIRONMENT_REJECTED' }

$maliciousCargoRoot = Join-Path $testRoot 'malicious-path-cargo'
[IO.Directory]::CreateDirectory($maliciousCargoRoot) | Out-Null
[IO.File]::WriteAllBytes((Join-Path $maliciousCargoRoot 'cargo.exe'), [byte[]](0x4d, 0x5a, 0x00, 0x00))
$originalPath = [Environment]::GetEnvironmentVariable('PATH', 'Process')
try {
    [Environment]::SetEnvironmentVariable('PATH', ($maliciousCargoRoot + [IO.Path]::PathSeparator + $originalPath), 'Process')
    $maliciousCargoRejected = $false
    try { $null = Get-ApprovedRustToolchainReceipt }
    catch { $maliciousCargoRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_BUILD_PATH_CARGO_REJECTED') }
    if (-not $maliciousCargoRejected) { throw 'TASK038_ACCEPT_TEST_MALICIOUS_PATH_CARGO_NOT_REJECTED' }
}
finally { [Environment]::SetEnvironmentVariable('PATH', $originalPath, 'Process') }

$originalWrapper = [Environment]::GetEnvironmentVariable('RUSTC_WRAPPER', 'Process')
try {
    [Environment]::SetEnvironmentVariable('RUSTC_WRAPPER', (Join-Path $maliciousCargoRoot 'wrapper.exe'), 'Process')
    $wrapperRejected = $false
    try { $null = Get-ApprovedRustToolchainReceipt }
    catch { $wrapperRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_BUILD_SUBSTITUTION_REJECTED') }
    if (-not $wrapperRejected) { throw 'TASK038_ACCEPT_TEST_MALICIOUS_WRAPPER_NOT_REJECTED' }
}
finally { [Environment]::SetEnvironmentVariable('RUSTC_WRAPPER', $originalWrapper, 'Process') }

$staleBinary = Join-Path $testRoot 'stale-target\debug\latticed.exe'
[IO.Directory]::CreateDirectory((Split-Path -Parent $staleBinary)) | Out-Null
[IO.File]::WriteAllBytes($staleBinary, [byte[]](0x4d, 0x5a, 0x00, 0x00))
$staleBinaryRejected = $false
try { Assert-CandidateBuildOutputAbsent -Path $staleBinary }
catch { $staleBinaryRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_CANDIDATE_BUILD_OUTPUT_NOT_FRESH') }
if (-not $staleBinaryRejected) { throw 'TASK038_ACCEPT_TEST_STALE_BUILD_OUTPUT_NOT_REJECTED' }

function Write-LifecycleFixture {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Mutation,
        [Parameter(Mandatory = $true)][string]$ExpectedExeSha256,
        [Parameter(Mandatory = $true)][string]$ExpectedExeNativeIdentity
    )

    [IO.Directory]::CreateDirectory($Root) | Out-Null
    $eventPath = Join-Path $Root 'events.jsonl'
    $outerSession = '1' * 32
    $rawSession = if ($Mutation -ceq 'session') { '2' * 32 } else { $outerSession }
    $outerConfig = '3' * 64
    $rawConfig = if ($Mutation -ceq 'config') { '4' * 64 } else { $outerConfig }
    $rawExe = if ($Mutation -ceq 'exe') { '5' * 64 } else { $ExpectedExeSha256 }
    $eventTypes = @('SPAWN', 'OPEN', 'CLOSE_REQUESTED', 'PIPE_CLOSED', 'EXITED', 'REAPED')
    if ($Mutation -ceq 'order') { $eventTypes[1] = 'PIPE_CLOSED' }
    $previous = '0' * 64
    $records = [Collections.Generic.List[object]]::new()
    $baseTime = [DateTimeOffset]::UtcNow.AddSeconds(-10)
    if ($Mutation -ceq 'stale') { $baseTime = [DateTimeOffset]::UtcNow.AddHours(-2) }
    for ($index = 0; $index -lt 6; $index++) {
        $observed = $baseTime.AddSeconds($index).ToString('yyyy-MM-ddTHH:mm:ss.fffffffZ', [Globalization.CultureInfo]::InvariantCulture)
        $exitCode = if ($index -lt 4) { $null } else { 0 }
        $exitText = if ($null -eq $exitCode) { 'null' } else { '0' }
        $identity = [ordered]@{
            pid = 12345
            creation_time = '134000000000000000'
            creation_time_source = 'WINDOWS_PROCESS_TIMES'
            exe_sha256 = $rawExe
        }
        $idempotency = Get-StringSha256 -Value (@(
            'lattice.tunnel-client.lifecycle-idempotency.v1', $rawSession, '7', $rawConfig,
            ('6' * 64), ('hmac-sha256:' + ('7' * 64)), $eventTypes[$index],
            '12345', '134000000000000000', 'WINDOWS_PROCESS_TIMES', $rawExe, $exitText
        ) -join "`n")
        $eventSha = Get-StringSha256 -Value (@(
            'lattice.tunnel-client.lifecycle-event-hash.v1', $previous, $idempotency,
            [string]($index + 1), $observed
        ) -join "`n")
        $record = [ordered]@{
            schema = $(if ($Mutation -ceq 'schema' -and $index -eq 0) { 'mutated' } else { 'lattice.tunnel-client.lifecycle-event.v1' })
            record_type = 'LIFECYCLE'
            component = 'mcpclient'
            event_type = $eventTypes[$index]
            session_id = $rawSession
            process_identity = $identity
            config_generation = 7
            safe_config_sha256 = $rawConfig
            session_command_sha256 = '6' * 64
            endpoint_ref = 'hmac-sha256:' + ('7' * 64)
            lifecycle_strategy = [ordered]@{
                transport = 'STDIO'; endpoint_kind = 'ANONYMOUS_PIPE'; spawn_mode = 'DIRECT'
                create_suspended_owned = $false; job_assignment_ownership = 'EXTERNAL_OWNER'
            }
            ordinal = $index + 1
            observed_at_utc = $observed
            exit_code = $exitCode
            previous_event_sha256 = $previous
            idempotency_key = $(if ($Mutation -ceq 'idempotency' -and $index -eq 2) { '0' * 64 } else { $idempotency })
            event_sha256 = $(if ($Mutation -ceq 'hash' -and $index -eq 3) { '0' * 64 } else { $eventSha })
            lifecycle_classification = 'UNKNOWN'
            threshold_profile_version = $null
            thresholds = [ordered]@{ pipe_milliseconds = $null; exit_milliseconds = $null; reap_milliseconds = $null; confirm_milliseconds = $null }
        }
        if ($Mutation -ceq 'extra-key' -and $index -eq 0) { $record['unexpected'] = $true }
        $records.Add($record)
        $previous = $eventSha
    }
    $eventText = [string]::Join("`n", @($records | ForEach-Object { $_ | ConvertTo-Json -Compress -Depth 12 })) + "`n"
    [IO.File]::WriteAllText($eventPath, $eventText, $script:Utf8)
    $eventIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $eventPath -Directory $false
    $profileIdentity = 'lattice.win-file-id.v1:0000000000000001:00000000000000000000000000000001:f'
    $tunnelIdentity = 'lattice.win-file-id.v1:0000000000000002:00000000000000000000000000000002:f'
    $latticedIdentity = $ExpectedExeNativeIdentity
    if ($Mutation -ceq 'repeated-component-identity') {
        $profileIdentity = $eventIdentity; $tunnelIdentity = $eventIdentity; $latticedIdentity = $eventIdentity
    }
    $outer = [ordered]@{
        schema = 'lattice.task038.tunnel-outer-lifecycle.v1'; mode = 'Run'; process_id = 22222; tunnel_client_exit_code = 0
        started_at_utc = $baseTime.ToString('yyyy-MM-ddTHH:mm:ss.fffffffZ', [Globalization.CultureInfo]::InvariantCulture)
        exited_at_utc = $baseTime.AddSeconds(6).ToString('yyyy-MM-ddTHH:mm:ss.fffffffZ', [Globalization.CultureInfo]::InvariantCulture)
        create_suspended = $true; job_assigned_before_resume = $true; descendant_processes_after_cleanup = 0
        profile_raw_sha256 = '8' * 64; profile_byte_count = 100; profile_strict_utf8 = $true; profile_native_identity = $profileIdentity
        latticed_native_identity = $latticedIdentity; tunnel_client_native_identity = $tunnelIdentity
        lifecycle_event_path = $eventPath; lifecycle_event_raw_sha256 = Get-FileSha256 -Path $eventPath
        lifecycle_event_byte_count = (Get-Item -LiteralPath $eventPath).Length; lifecycle_event_strict_utf8 = $true
        lifecycle_event_native_identity = $eventIdentity; lifecycle_session_id = $outerSession; lifecycle_config_generation = 7
        lifecycle_safe_config_schema = 'lattice.task038.tunnel-safe-config.v1'; lifecycle_safe_config_sha256 = $outerConfig
        lifecycle_safe_config_byte_count = 100; lifecycle_event_count = 6; lifecycle_anomaly_count = 0; lifecycle_anomaly_codes = @()
        lifecycle_chain_complete = $true; lifecycle_normal_close_complete = $true
        lifecycle_final_event_sha256 = $(if ($Mutation -ceq 'outer-hash') { '9' * 64 } else { $previous })
        lifecycle_inner_process_id = 12345; lifecycle_inner_process_creation_time = '134000000000000000'
        lifecycle_inner_process_creation_time_source = 'WINDOWS_PROCESS_TIMES'; lifecycle_inner_process_exe_sha256 = $rawExe
        lifecycle_inner_exit_code = 0; lifecycle_threshold_decision = 'C_CALIBRATION_FIRST'; lifecycle_threshold_profile = $null
        lifecycle_thresholds = [ordered]@{ pipe_milliseconds = $null; exit_milliseconds = $null; reap_milliseconds = $null; confirm_milliseconds = $null }
        lifecycle_classification = 'UNKNOWN'; leak_claimed = $false
    }
    $receiptPath = Join-Path $Root 'outer.json'
    [IO.File]::WriteAllText($receiptPath, (($outer | ConvertTo-Json -Compress -Depth 12) + "`n"), $script:Utf8)
    return $receiptPath
}

$expectedLifecycleExe = 'd' * 64
$expectedLifecycleNativeIdentity = 'lattice.win-file-id.v1:0000000000000003:00000000000000000000000000000003:f'
$validLifecyclePath = Write-LifecycleFixture -Root (Join-Path $testRoot 'lifecycle-valid') -Mutation 'none' -ExpectedExeSha256 $expectedLifecycleExe -ExpectedExeNativeIdentity $expectedLifecycleNativeIdentity
$validLifecycle = Read-TunnelLifecycleReceipt -Path $validLifecyclePath -ExpectedInnerExeSha256 $expectedLifecycleExe -ExpectedInnerExeNativeIdentity $expectedLifecycleNativeIdentity
if (
    -not [bool]$validLifecycle.exact_schema_hash_order_idempotency_session_config_exe_checked -or
    [string]$validLifecycle.authority_scope -cne 'NON_LIVE_PARSER_ONLY' -or
    [bool]$validLifecycle.live_authority_evaluated
) {
    throw 'TASK038_ACCEPT_TEST_LIFECYCLE_VALID_REJECTED'
}
$liveLifecycleRejected = $false
try { Assert-LiveTunnelLifecycleAuthority -ParsedReceipt $validLifecycle }
catch { $liveLifecycleRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_TUNNEL_LIFECYCLE_AUTHORITY_NOT_AVAILABLE') }
if (-not $liveLifecycleRejected) { throw 'TASK038_ACCEPT_TEST_PARSER_ONLY_LIFECYCLE_GRANTED_LIVE_AUTHORITY' }
foreach ($mutation in @('schema', 'order', 'session', 'config', 'exe', 'idempotency', 'hash', 'outer-hash', 'extra-key', 'repeated-component-identity', 'stale')) {
    $path = Write-LifecycleFixture -Root (Join-Path $testRoot ('lifecycle-' + $mutation)) -Mutation $mutation -ExpectedExeSha256 $expectedLifecycleExe -ExpectedExeNativeIdentity $expectedLifecycleNativeIdentity
    $rejected = $false
    try { $null = Read-TunnelLifecycleReceipt -Path $path -ExpectedInnerExeSha256 $expectedLifecycleExe -ExpectedInnerExeNativeIdentity $expectedLifecycleNativeIdentity }
    catch { $rejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_TUNNEL_LIFECYCLE_RECEIPT_REJECTED') }
    if (-not $rejected) { throw ('TASK038_ACCEPT_TEST_LIFECYCLE_MUTATION_NOT_REJECTED_' + $mutation) }
}

function Write-DispatchFixture {
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)][string]$Mutation)

    [IO.Directory]::CreateDirectory($Root) | Out-Null
    $path = Join-Path $Root 'dispatch.jsonl'
    $expectedSession = 'a' * 32
    $rawSession = if ($Mutation -ceq 'session') { 'b' * 32 } else { $expectedSession }
    $expectedConfig = 'c' * 64
    $rawConfig = if ($Mutation -ceq 'config') { 'd' * 64 } else { $expectedConfig }
    $types = @('SESSION_OPEN', 'DISPATCH_ACCEPTED', 'SESSION_CLOSED')
    if ($Mutation -ceq 'order') { $types[0] = 'SESSION_CLOSED' }
    $previous = '0' * 64
    $records = [Collections.Generic.List[object]]::new()
    for ($index = 0; $index -lt 3; $index++) {
        $isDispatch = $index -eq 1
        $count = if ($isDispatch) { 1 } elseif ($index -eq 2) { 1 } else { 0 }
        $tool = if ($isDispatch) { $(if ($Mutation -ceq 'tool') { 'lattice_delivery_run' } else { 'lattice_task_status' }) } else { $null }
        $request = if ($isDispatch) { 'e' * 64 } else { $null }
        $hash = Get-StringSha256 -Value (@(
            'lattice.mcp.acceptance-dispatch-hash.v1', $previous, $rawSession, $rawConfig,
            $types[$index], [string]($index + 1), '12345', $(if ($null -eq $tool) { 'null' } else { $tool }),
            $(if ($null -eq $request) { 'null' } else { $request }), [string]$count, [string](1000 + $index)
        ) -join "`n")
        $record = [ordered]@{
            schema = $(if ($Mutation -ceq 'schema' -and $index -eq 0) { 'mutated' } else { 'lattice.mcp.acceptance-dispatch.v1' })
            record_type = $types[$index]; session_id = $rawSession; safe_config_sha256 = $rawConfig
            process_id = 12345; ordinal = $index + 1; tool_name = $tool; request_id_sha256 = $request
            dispatch_accepted_count = $count; observed_at_unix_nanos = [string](1000 + $index)
            previous_event_sha256 = $previous
            event_sha256 = $(if ($Mutation -ceq 'hash' -and $index -eq 1) { '0' * 64 } else { $hash })
        }
        if ($Mutation -ceq 'extra-key' -and $index -eq 0) { $record['unexpected'] = $true }
        $records.Add($record)
        $previous = $hash
    }
    if ($Mutation -ceq 'missing-close') { $records.RemoveAt(2) }
    [IO.File]::WriteAllText($path, ([string]::Join("`n", @($records | ForEach-Object { $_ | ConvertTo-Json -Compress -Depth 8 })) + "`n"), $script:Utf8)
    return [ordered]@{
        sink = [ordered]@{
            path = $path
            native_identity = Get-AuthoritativeNativeFileIdentity -Path $path
            root_path = $Root
            root_native_identity = Get-LatticeWindowsNativePathIdentityToken -Path $Root -Directory $true
        }
        session = $expectedSession
        config = $expectedConfig
    }
}

$validDispatch = Write-DispatchFixture -Root (Join-Path $testRoot 'dispatch-valid') -Mutation 'none'
$validDispatchReceipt = Read-McpAcceptanceEvidence -Sink $validDispatch.sink -SessionId $validDispatch.session -SafeConfigSha256 $validDispatch.config -ProcessId 12345 -ExpectedDispatchTools @('lattice_task_status')
if ([int]$validDispatchReceipt.dispatch_accepted_count -ne 1 -or -not [bool]$validDispatchReceipt.normal_close_complete) {
    throw 'TASK038_ACCEPT_TEST_DISPATCH_VALID_REJECTED'
}
foreach ($mutation in @('schema', 'order', 'session', 'config', 'tool', 'hash', 'extra-key', 'missing-close')) {
    $fixture = Write-DispatchFixture -Root (Join-Path $testRoot ('dispatch-' + $mutation)) -Mutation $mutation
    $rejected = $false
    try { $null = Read-McpAcceptanceEvidence -Sink $fixture.sink -SessionId $fixture.session -SafeConfigSha256 $fixture.config -ProcessId 12345 -ExpectedDispatchTools @('lattice_task_status') }
    catch { $rejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_MCP_EVIDENCE_REJECTED') }
    if (-not $rejected) { throw ('TASK038_ACCEPT_TEST_DISPATCH_MUTATION_NOT_REJECTED_' + $mutation) }
}

$footprintRoot = Join-Path $testRoot 'effect-footprint'
[IO.Directory]::CreateDirectory($footprintRoot) | Out-Null
$footprintFile = Join-Path $footprintRoot 'probe.txt'
[IO.File]::WriteAllText($footprintFile, 'before', $script:Utf8)
$footprintBefore = Get-DirectoryFootprint -Root $footprintRoot
[IO.File]::WriteAllText($footprintFile, 'after', $script:Utf8)
$footprintAfter = Get-DirectoryFootprint -Root $footprintRoot
if ([string]$footprintBefore.digest -ceq [string]$footprintAfter.digest) {
    throw 'TASK038_ACCEPT_TEST_EFFECT_FOOTPRINT_MUTATION_NOT_OBSERVED'
}

$deliveryFixtureRoot = Join-Path $testRoot 'delivery-effect'
$deliveryFixtureRepo = Join-Path $deliveryFixtureRoot 'repo'
[IO.Directory]::CreateDirectory($deliveryFixtureRepo) | Out-Null
$null = & git -C $deliveryFixtureRepo init --quiet
$null = & git -C $deliveryFixtureRepo config user.name 'TASK038 Test'
$null = & git -C $deliveryFixtureRepo config user.email 'task038@example.invalid'
$null = & git -c commit.gpgsign=false -C $deliveryFixtureRepo commit --quiet --allow-empty -m 'initial'
[IO.File]::WriteAllBytes((Join-Path $deliveryFixtureRepo 'answer.txt'), [Text.Encoding]::ASCII.GetBytes("LATTICE_DELIVERY_OK`n"))
$null = & git -C $deliveryFixtureRepo add -- answer.txt
$null = & git -c commit.gpgsign=false -C $deliveryFixtureRepo commit --quiet -m 'bounded answer'
if ($LASTEXITCODE -ne 0) { throw 'TASK038_ACCEPT_TEST_DELIVERY_EFFECT_FIXTURE_REJECTED' }
$script:ProductionDeliveryRoot = $deliveryFixtureRoot
$deliveryEffect = Get-DeliveryGitEffectReceipt
if ([string]$deliveryEffect.changed_path -cne 'answer.txt' -or [int]$deliveryEffect.commit_count -ne 2) {
    throw 'TASK038_ACCEPT_TEST_DELIVERY_EFFECT_RECEIPT_REJECTED'
}
$effectSession = [pscustomobject]@{
    AcceptanceEvidence = [pscustomobject]@{
        schema = 'lattice.task038.mcp-acceptance-dispatch-evidence.v1'
        session_id = 'f' * 32; raw_sha256 = '1' * 64; final_event_sha256 = '2' * 64
        dispatch_accepted_count = 3; normal_close_complete = $true
    }
    Summary = [pscustomobject]@{
        job_active_processes_after_exit = 0; process_session_pid_present_after_cleanup = 0
        network_tcp_owner_rows_after_cleanup = 0; network_udp_owner_rows_after_cleanup = 0
    }
}
$sourceEffect = [ordered]@{ head = '3' * 40; tree = '4' * 40; status_sha256 = '5' * 64; status_empty = $true }
$submitEffectBefore = [ordered]@{
    source_git = $sourceEffect
    codex_home = [ordered]@{ root_native_identity = 'codex-root'; digest = '6' * 64 }
    delivery_root = [ordered]@{ root_native_identity = 'delivery-root'; digest = '7' * 64 }
    database = [ordered]@{ codex_intents = 0 }
}
$submitEffectAfter = [ordered]@{
    source_git = $sourceEffect
    codex_home = [ordered]@{ root_native_identity = 'codex-root'; digest = '8' * 64 }
    delivery_root = [ordered]@{ root_native_identity = 'delivery-root'; digest = '9' * 64 }
    database = [ordered]@{ codex_intents = 1 }
}
$submitEffect = New-SubmitProductionSessionEffectReceipt -Before $submitEffectBefore -After $submitEffectAfter -Session $effectSession
if (-not [bool]$submitEffect.exact_bounded_effect_observed) { throw 'TASK038_ACCEPT_TEST_SUBMIT_EFFECT_RECEIPT_REJECTED' }
$effectSession.AcceptanceEvidence.dispatch_accepted_count = 2
$submitMutationRejected = $false
try { $null = New-SubmitProductionSessionEffectReceipt -Before $submitEffectBefore -After $submitEffectAfter -Session $effectSession }
catch { $submitMutationRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_PRODUCTION_SESSION_EFFECT_REJECTED') }
if (-not $submitMutationRejected) { throw 'TASK038_ACCEPT_TEST_SUBMIT_EFFECT_MUTATION_NOT_REJECTED' }
$effectSession.AcceptanceEvidence.dispatch_accepted_count = 2
$snapshotOnlyRejected = $false
try { $null = New-ZeroProductionSessionEffectReceipt -Phase 'read-only' -Before $submitEffectBefore -After $submitEffectBefore -Session $effectSession }
catch { $snapshotOnlyRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_PRODUCTION_SESSION_EFFECT_AUTHORITY_NOT_AVAILABLE') }
if (-not $snapshotOnlyRejected) { throw 'TASK038_ACCEPT_TEST_SNAPSHOT_ONLY_ZERO_GRANTED_AUTHORITY' }
$transientRoot = Join-Path $testRoot 'transient-effect-disappears-before-snapshot'
[IO.Directory]::CreateDirectory($transientRoot) | Out-Null
$transientBefore = Get-DirectoryFootprint -Root $transientRoot
$transientFile = Join-Path $transientRoot 'transient.txt'
[IO.File]::WriteAllText($transientFile, 'transient', $script:Utf8)
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$transientProcess = [Diagnostics.Process]::Start((Join-Path $PSHOME 'powershell.exe'), '-NoLogo -NoProfile -NonInteractive -Command "exit 0"')
if ($null -eq $transientProcess -or -not $transientProcess.WaitForExit(10000) -or $transientProcess.ExitCode -ne 0) {
    throw 'TASK038_ACCEPT_TEST_TRANSIENT_PROCESS_FIXTURE_REJECTED'
}
$transientProcess.Dispose()
$listener.Stop()
Remove-Item -LiteralPath $transientFile -Force
$transientAfter = Get-DirectoryFootprint -Root $transientRoot
if ((ConvertTo-CanonicalJson -Value $transientBefore) -cne (ConvertTo-CanonicalJson -Value $transientAfter)) {
    throw 'TASK038_ACCEPT_TEST_TRANSIENT_EFFECT_FIXTURE_NOT_EPHEMERAL'
}
$transientAuthorityRejected = $false
try { $null = New-ZeroProductionSessionEffectReceipt -Phase 'transient-effect-regression' -Before $transientBefore -After $transientAfter -Session $effectSession }
catch { $transientAuthorityRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_PRODUCTION_SESSION_EFFECT_AUTHORITY_NOT_AVAILABLE') }
if (-not $transientAuthorityRejected) { throw 'TASK038_ACCEPT_TEST_TRANSIENT_EFFECT_DISAPPEARANCE_NOT_REJECTED' }
$zeroMutationRejected = $false
try { $null = New-ZeroProductionSessionEffectReceipt -Phase 'read-only' -Before $submitEffectBefore -After $submitEffectAfter -Session $effectSession }
catch { $zeroMutationRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_PRODUCTION_SESSION_EFFECT_REJECTED') }
if (-not $zeroMutationRejected) { throw 'TASK038_ACCEPT_TEST_ZERO_EFFECT_MUTATION_NOT_REJECTED' }
$effectSession.AcceptanceEvidence.dispatch_accepted_count = 0
$missingNegativeAuthorityRejected = $false
try {
    $null = Assert-ProductionNegativeEffectAuthority `
        -Session $effectSession `
        -ProbeName '02n01-unknown-tool' `
        -ExpectedRejectedClassification 'MCP_INVALID_PARAMS'
}
catch { $missingNegativeAuthorityRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_PRODUCTION_NEGATIVE_EFFECT_AUTHORITY_NOT_AVAILABLE') }
if (-not $missingNegativeAuthorityRejected) { throw 'TASK038_ACCEPT_TEST_MISSING_NEGATIVE_EFFECT_AUTHORITY_NOT_REJECTED' }
foreach ($reservedPort in @(5432, 64272, 55432)) {
    $rejected = $false
    try { Assert-PostgresPortPolicy -Port $reservedPort }
    catch { $rejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_POSTGRES_PORT_REJECTED') }
    if (-not $rejected) { throw ('TASK038_ACCEPT_TEST_RESERVED_PORT_NOT_REJECTED_' + $reservedPort) }
}
$nonDynamicPortRejected = $false
try { Assert-PostgresPortPolicy -Port 1025 }
catch { $nonDynamicPortRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_POSTGRES_PORT_REJECTED') }
if (-not $nonDynamicPortRejected) { throw 'TASK038_ACCEPT_TEST_NON_DYNAMIC_PORT_NOT_REJECTED' }
$acceptanceStartedAt = [DateTimeOffset]::UtcNow.AddSeconds(-20)
$markerCreatedAt = $acceptanceStartedAt.AddSeconds(-10)
$initialStartedAt = $acceptanceStartedAt.AddSeconds(-8)
$restartStartedAt = $acceptanceStartedAt.AddSeconds(2)
$observedAt = $acceptanceStartedAt.AddSeconds(10)
$chronology = Assert-PostgresRestartChronology `
    -MarkerCreatedAtUtc $markerCreatedAt.ToString('o') `
    -InitialPostmasterStartedAt $initialStartedAt.ToString('o') `
    -RestartPostmasterStartedAt $restartStartedAt.ToString('o') `
    -AcceptanceStartedAtUtc $acceptanceStartedAt `
    -ObservedAtUtc $observedAt
if (-not [bool]$chronology.restart_precedes_acceptance_protocol) {
    throw 'TASK038_ACCEPT_TEST_POSTGRES_RESTART_CHRONOLOGY_VALID_REJECTED'
}
$reusedPostmasterRejected = $false
try {
    $null = Assert-PostgresRestartChronology `
        -MarkerCreatedAtUtc $acceptanceStartedAt.ToString('o') `
        -InitialPostmasterStartedAt $acceptanceStartedAt.AddHours(-2).ToString('o') `
        -RestartPostmasterStartedAt $acceptanceStartedAt.AddHours(-1).ToString('o') `
        -AcceptanceStartedAtUtc $acceptanceStartedAt `
        -ObservedAtUtc $observedAt
}
catch { $reusedPostmasterRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_POSTGRES_RESTART_CHRONOLOGY_REJECTED') }
if (-not $reusedPostmasterRejected) { throw 'TASK038_ACCEPT_TEST_REUSED_POSTMASTER_RECENT_MARKER_NOT_REJECTED' }
$holderReceiptRejected = $false
try { Assert-PostgresHolderReceiptAuthority -Binding ([pscustomobject]@{ holder_restart_receipt_authority = 'NOT_AVAILABLE' }) }
catch { $holderReceiptRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_POSTGRES_HOLDER_RECEIPT_NOT_AVAILABLE') }
if (-not $holderReceiptRejected) { throw 'TASK038_ACCEPT_TEST_POSTGRES_HOLDER_RECEIPT_NOT_REQUIRED' }
$fakeBinary = Join-Path $testRoot 'latticed-fake.exe'
$fakeState = Join-Path $testRoot 'state.txt'
$fakeCounters = Join-Path $testRoot 'counters.txt'
$script:HarnessObservedCounterPath = $fakeCounters
$descendantPid = Join-Path $testRoot 'descendant-pid.txt'
$source = @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Text.RegularExpressions;
using System.Threading;
using System.Web.Script.Serialization;

public static class Task038StrictFakeMcp
{
    private static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
    private static readonly string TaskRef = new string('a', 64);
    private static readonly string Ledger = new string('b', 64);
    private static readonly string Result = new string('c', 64);
    private static readonly string StatusJson = "{\"ledger_head_digest\":\"" + Ledger + "\",\"result_digest\":\"" + Result + "\",\"schema_version\":\"lattice.task.status.v1\",\"status\":\"COMPLETED\",\"task_ref\":\"" + TaskRef + "\",\"task_state\":\"COMPLETED\"}";
    private static readonly Regex ClientRequestId = new Regex("^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$", RegexOptions.CultureInvariant);
    private static readonly Regex TaskReference = new Regex("^[0-9a-f]{64}$", RegexOptions.CultureInvariant);

    private static string Response(int id, string result) { return "{\"jsonrpc\":\"2.0\",\"id\":" + id + ",\"result\":" + result + "}"; }
    private static string Error(int id, int code, string message) { return "{\"jsonrpc\":\"2.0\",\"id\":" + id + ",\"error\":{\"code\":" + code + ",\"message\":" + Json.Serialize(message) + "}}"; }
    private static string Meta() { return "{\"io.modelcontextprotocol/serverInfo\":{\"name\":\"latticed\",\"title\":\"LATTICE DevOS\",\"version\":\"1.0.0\"}}"; }

    private static string ToolResult(int id, bool isError, string structured, bool modern)
    {
        string fields = "\"content\":[{\"type\":\"text\",\"text\":" + Json.Serialize(structured) + "}],\"isError\":" + (isError ? "true" : "false") + ",\"structuredContent\":" + structured;
        if (modern) fields = "\"_meta\":" + Meta() + "," + fields + ",\"resultType\":\"complete\"";
        return Response(id, "{" + fields + "}");
    }

    private static string ReplaceRequired(string value, string oldValue, string newValue)
    {
        if (!value.Contains(oldValue)) throw new InvalidOperationException("mutation target missing");
        return value.Replace(oldValue, newValue);
    }

    private static string DeliverySchema()
    {
        string value = "{\"type\":\"object\",\"additionalProperties\":false}";
        string mutation = Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_SCHEMA_MUTATION");
        if (mutation == "delivery-type") value = ReplaceRequired(value, "\"type\":\"object\"", "\"type\":\"array\"");
        if (mutation == "delivery-additional") value = ReplaceRequired(value, "\"additionalProperties\":false", "\"additionalProperties\":true");
        if (mutation == "delivery-properties") value = ReplaceRequired(value, "\"additionalProperties\":false", "\"properties\":{},\"additionalProperties\":false");
        return value;
    }

    private static string SubmitSchema()
    {
        string value = "{\"type\":\"object\",\"properties\":{\"client_request_id\":{\"type\":\"string\",\"minLength\":1,\"maxLength\":64,\"pattern\":\"^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$\"},\"intent\":{\"type\":\"string\",\"enum\":[\"CONTROLLED_CODEX_CANARY\"]}},\"required\":[\"client_request_id\",\"intent\"],\"additionalProperties\":false}";
        string m = Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_SCHEMA_MUTATION");
        if (m == "submit-type") value = ReplaceRequired(value, "\"type\":\"object\"", "\"type\":\"array\"");
        if (m == "submit-required") value = ReplaceRequired(value, "[\"client_request_id\",\"intent\"]", "[\"client_request_id\"]");
        if (m == "submit-additional") value = ReplaceRequired(value, "\"additionalProperties\":false", "\"additionalProperties\":true");
        if (m == "submit-client-type") value = ReplaceRequired(value, "\"client_request_id\":{\"type\":\"string\"", "\"client_request_id\":{\"type\":\"number\"");
        if (m == "submit-client-min") value = ReplaceRequired(value, "\"minLength\":1", "\"minLength\":2");
        if (m == "submit-client-max") value = ReplaceRequired(value, "\"maxLength\":64", "\"maxLength\":63");
        if (m == "submit-client-pattern") value = ReplaceRequired(value, "{0,63}$", "{0,62}$");
        if (m == "submit-intent-type") value = ReplaceRequired(value, "\"intent\":{\"type\":\"string\"", "\"intent\":{\"type\":\"number\"");
        if (m == "submit-intent-enum") value = ReplaceRequired(value, "CONTROLLED_CODEX_CANARY", "UNKNOWN_INTENT");
        if (m == "submit-extra-property") value = ReplaceRequired(value, "\"intent\":{", "\"shell\":{\"type\":\"string\"},\"intent\":{");
        return value;
    }

    private static string TaskStatusSchema()
    {
        string value = "{\"type\":\"object\",\"properties\":{\"task_ref\":{\"type\":\"string\",\"minLength\":64,\"maxLength\":64,\"pattern\":\"^[0-9a-f]{64}$\"}},\"required\":[\"task_ref\"],\"additionalProperties\":false}";
        string m = Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_SCHEMA_MUTATION");
        if (m == "status-type") value = ReplaceRequired(value, "\"type\":\"object\"", "\"type\":\"array\"");
        if (m == "status-required") value = ReplaceRequired(value, "[\"task_ref\"]", "[]");
        if (m == "status-additional") value = ReplaceRequired(value, "\"additionalProperties\":false", "\"additionalProperties\":true");
        if (m == "status-ref-type") value = ReplaceRequired(value, "\"task_ref\":{\"type\":\"string\"", "\"task_ref\":{\"type\":\"number\"");
        if (m == "status-ref-min") value = ReplaceRequired(value, "\"minLength\":64", "\"minLength\":63");
        if (m == "status-ref-max") value = ReplaceRequired(value, "\"maxLength\":64", "\"maxLength\":65");
        if (m == "status-ref-pattern") value = ReplaceRequired(value, "^[0-9a-f]{64}$", "^[0-9A-F]{64}$");
        if (m == "status-extra-property") value = ReplaceRequired(value, "\"task_ref\":{", "\"ref\":{\"type\":\"string\"},\"task_ref\":{");
        return value;
    }

    private static string OutputSchema()
    {
        string value = "{\"type\":\"object\",\"properties\":{\"schema_version\":{\"type\":\"string\",\"enum\":[\"lattice.task.status.v1\"]},\"status\":{\"type\":\"string\",\"enum\":[\"NOT_SUBMITTED\",\"RECONCILIATION_REQUIRED\",\"FAILED\",\"COMPLETED\"]},\"task_state\":{\"type\":\"string\",\"enum\":[\"NOT_SUBMITTED\",\"DRAFT\",\"AWAITING_EXECUTION_APPROVAL\",\"PREPARING\",\"EXECUTING\",\"VERIFYING\",\"REVIEWING\",\"AWAITING_MERGE_APPROVAL\",\"MERGING\",\"COMPLETED\",\"REJECTED\",\"BLOCKED\",\"FAILED\",\"STOPPING\",\"CANCELLED\"]},\"task_ref\":{\"type\":\"string\",\"minLength\":64,\"maxLength\":64,\"pattern\":\"^[0-9a-f]{64}$\"},\"ledger_head_digest\":{\"type\":\"string\",\"minLength\":64,\"maxLength\":64,\"pattern\":\"^[0-9a-f]{64}$\"},\"result_digest\":{\"anyOf\":[{\"type\":\"string\",\"minLength\":64,\"maxLength\":64,\"pattern\":\"^[0-9a-f]{64}$\"},{\"type\":\"null\"}]}},\"required\":[\"schema_version\",\"status\",\"task_state\",\"task_ref\",\"ledger_head_digest\",\"result_digest\"],\"additionalProperties\":false}";
        string m = Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_SCHEMA_MUTATION");
        if (m == "output-type") value = ReplaceRequired(value, "\"type\":\"object\"", "\"type\":\"array\"");
        if (m == "output-required") value = ReplaceRequired(value, ",\"result_digest\"]", "]");
        if (m == "output-additional") value = ReplaceRequired(value, "\"additionalProperties\":false", "\"additionalProperties\":true");
        if (m == "output-schema-type") value = ReplaceRequired(value, "\"schema_version\":{\"type\":\"string\"", "\"schema_version\":{\"type\":\"number\"");
        if (m == "output-schema-enum") value = ReplaceRequired(value, "lattice.task.status.v1", "lattice.task.status.v2");
        if (m == "output-status-type") value = ReplaceRequired(value, "\"status\":{\"type\":\"string\"", "\"status\":{\"type\":\"number\"");
        if (m == "output-status-enum") value = ReplaceRequired(value, "\"COMPLETED\"]},\"task_state\"", "\"UNKNOWN\"]},\"task_state\"");
        if (m == "output-state-type") value = ReplaceRequired(value, "\"task_state\":{\"type\":\"string\"", "\"task_state\":{\"type\":\"number\"");
        if (m == "output-state-enum") value = ReplaceRequired(value, "\"CANCELLED\"]},\"task_ref\"", "\"UNKNOWN\"]},\"task_ref\"");
        if (m == "output-task-type") value = ReplaceRequired(value, "\"task_ref\":{\"type\":\"string\"", "\"task_ref\":{\"type\":\"number\"");
        if (m == "output-task-min") value = ReplaceRequired(value, "\"task_ref\":{\"type\":\"string\",\"minLength\":64", "\"task_ref\":{\"type\":\"string\",\"minLength\":63");
        if (m == "output-task-max") value = ReplaceRequired(value, "\"task_ref\":{\"type\":\"string\",\"minLength\":64,\"maxLength\":64", "\"task_ref\":{\"type\":\"string\",\"minLength\":64,\"maxLength\":65");
        if (m == "output-task-pattern") value = ReplaceRequired(value, "\"task_ref\":{\"type\":\"string\",\"minLength\":64,\"maxLength\":64,\"pattern\":\"^[0-9a-f]{64}$\"", "\"task_ref\":{\"type\":\"string\",\"minLength\":64,\"maxLength\":64,\"pattern\":\"^[0-9A-F]{64}$\"");
        if (m == "output-ledger-type") value = ReplaceRequired(value, "\"ledger_head_digest\":{\"type\":\"string\"", "\"ledger_head_digest\":{\"type\":\"number\"");
        if (m == "output-ledger-bounds") value = ReplaceRequired(value, "\"ledger_head_digest\":{\"type\":\"string\",\"minLength\":64,\"maxLength\":64", "\"ledger_head_digest\":{\"type\":\"string\",\"minLength\":63,\"maxLength\":65");
        if (m == "output-ledger-pattern") value = ReplaceRequired(value, "\"ledger_head_digest\":{\"type\":\"string\",\"minLength\":64,\"maxLength\":64,\"pattern\":\"^[0-9a-f]{64}$\"", "\"ledger_head_digest\":{\"type\":\"string\",\"minLength\":64,\"maxLength\":64,\"pattern\":\"^[0-9A-F]{64}$\"");
        if (m == "output-result-anyof") value = ReplaceRequired(value, ",{\"type\":\"null\"}]", "]");
        if (m == "output-result-string-type") value = ReplaceRequired(value, "\"result_digest\":{\"anyOf\":[{\"type\":\"string\"", "\"result_digest\":{\"anyOf\":[{\"type\":\"number\"");
        if (m == "output-result-bounds") value = ReplaceRequired(value, "\"result_digest\":{\"anyOf\":[{\"type\":\"string\",\"minLength\":64,\"maxLength\":64", "\"result_digest\":{\"anyOf\":[{\"type\":\"string\",\"minLength\":63,\"maxLength\":65");
        if (m == "output-result-pattern") value = ReplaceRequired(value, "\"result_digest\":{\"anyOf\":[{\"type\":\"string\",\"minLength\":64,\"maxLength\":64,\"pattern\":\"^[0-9a-f]{64}$\"", "\"result_digest\":{\"anyOf\":[{\"type\":\"string\",\"minLength\":64,\"maxLength\":64,\"pattern\":\"^[0-9A-F]{64}$\"");
        if (m == "output-result-null-type") value = ReplaceRequired(value, "{\"type\":\"null\"}", "{\"type\":\"string\"}");
        return value;
    }

    private static string Tools()
    {
        string delivery = DeliverySchema();
        string deliveryRun = "{\"name\":\"lattice_delivery_run\",\"inputSchema\":" + delivery;
        if (Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_SCHEMA_MUTATION") == "delivery-output") deliveryRun += ",\"outputSchema\":{}";
        deliveryRun += "}";
        string firstTwo = deliveryRun + ",{\"name\":\"lattice_delivery_status\",\"inputSchema\":" + delivery + "}";
        if (Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_TOOL_SET_MUTATION") == "two-tools") return "[" + firstTwo + "]";
        if (Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_TOOL_SET_MUTATION") == "fifth-tool") return "[" + firstTwo + ",{\"name\":\"lattice_extra\",\"inputSchema\":" + delivery + "},{\"name\":\"lattice_task_submit\",\"inputSchema\":" + SubmitSchema() + ",\"outputSchema\":" + OutputSchema() + "},{\"name\":\"lattice_task_status\",\"inputSchema\":" + TaskStatusSchema() + ",\"outputSchema\":" + OutputSchema() + "}]";
        return "[" + firstTwo + ",{\"name\":\"lattice_task_submit\",\"inputSchema\":" + SubmitSchema() + ",\"outputSchema\":" + OutputSchema() + "},{\"name\":\"lattice_task_status\",\"inputSchema\":" + TaskStatusSchema() + ",\"outputSchema\":" + OutputSchema() + "}]";
    }

    private static Dictionary<string, object> Object(object value) { return value as Dictionary<string, object>; }
    private static bool ExactKeys(Dictionary<string, object> value, params string[] keys)
    {
        if (value == null || value.Count != keys.Length) return false;
        foreach (string key in keys) if (!value.ContainsKey(key)) return false;
        return true;
    }
    private static bool Modern(Dictionary<string, object> parameters)
    {
        object metaValue;
        if (parameters == null || !parameters.TryGetValue("_meta", out metaValue)) return false;
        Dictionary<string, object> meta = Object(metaValue);
        return meta != null && meta.ContainsKey("io.modelcontextprotocol/protocolVersion");
    }

    private static void Increment(string tool, bool effect)
    {
        string path = Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_COUNTERS");
        if (String.IsNullOrEmpty(path)) throw new InvalidOperationException("counter path missing");
        Dictionary<string, int> values = new Dictionary<string, int>(StringComparer.Ordinal);
        foreach (string key in new [] { "dispatch", "effect", "delivery_run", "delivery_status", "task_submit", "task_status" }) values[key] = 0;
        if (File.Exists(path)) foreach (string line in File.ReadAllLines(path))
        {
            string[] parts = line.Split('=');
            int parsed;
            if (parts.Length == 2 && values.ContainsKey(parts[0]) && Int32.TryParse(parts[1], out parsed)) values[parts[0]] = parsed;
        }
        values["dispatch"]++;
        values[tool]++;
        if (effect) values["effect"]++;
        List<string> output = new List<string>();
        foreach (string key in new [] { "dispatch", "effect", "delivery_run", "delivery_status", "task_submit", "task_status" }) output.Add(key + "=" + values[key]);
        File.WriteAllLines(path, output.ToArray());
    }

    private static void MutateRejectedCall(string tool)
    {
        if (Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_NEGATIVE_EFFECT") == "1") Increment(tool, true);
    }

    private static bool DeliveryArguments(object value) { Dictionary<string, object> arguments = Object(value); return arguments != null && arguments.Count == 0; }
    private static bool SubmitArguments(object value, out string requestId)
    {
        requestId = null;
        Dictionary<string, object> arguments = Object(value);
        if (!ExactKeys(arguments, "client_request_id", "intent")) return false;
        object idValue, intentValue;
        if (!arguments.TryGetValue("client_request_id", out idValue) || !arguments.TryGetValue("intent", out intentValue)) return false;
        requestId = idValue as string;
        return requestId != null && ClientRequestId.IsMatch(requestId) && (intentValue as string) == "CONTROLLED_CODEX_CANARY";
    }
    private static bool StatusArguments(object value, out string taskRef)
    {
        taskRef = null;
        Dictionary<string, object> arguments = Object(value);
        if (!ExactKeys(arguments, "task_ref")) return false;
        object reference;
        if (!arguments.TryGetValue("task_ref", out reference)) return false;
        taskRef = reference as string;
        return taskRef != null && TaskReference.IsMatch(taskRef);
    }

    private static string ErrorStructured(string code)
    {
        string mutation = Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_ERROR_MUTATION");
        if (mutation == "extra-key") return "{\"status\":\"ERROR\",\"code\":" + Json.Serialize(code) + ",\"detail\":\"fixed\"}";
        return "{\"status\":\"ERROR\",\"code\":" + Json.Serialize(code) + "}";
    }

    private static void SpawnDescendantMutation()
    {
        if (Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_DESCENDANT_ESCAPE") != "1") return;
        ProcessStartInfo start = new ProcessStartInfo(Process.GetCurrentProcess().MainModule.FileName);
        start.UseShellExecute = false;
        start.CreateNoWindow = true;
        start.RedirectStandardInput = true;
        start.RedirectStandardOutput = true;
        start.RedirectStandardError = true;
        start.EnvironmentVariables["TASK038_ACCEPT_FAKE_DESCENDANT_CHILD"] = "1";
        Process child = Process.Start(start);
        File.WriteAllText(Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_DESCENDANT_PID"), child.Id.ToString());
    }

    public static int Main()
    {
        if (Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_DESCENDANT_CHILD") == "1") { Thread.Sleep(60000); return 0; }
        SpawnDescendantMutation();
        string state = Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_STATE");
        string line;
        while ((line = Console.ReadLine()) != null)
        {
            Dictionary<string, object> request;
            try { request = Object(Json.DeserializeObject(line)); }
            catch { continue; }
            if (request == null || !request.ContainsKey("id")) continue;
            int id = Convert.ToInt32(request["id"]);
            string method = request.ContainsKey("method") ? request["method"] as string : null;
            Dictionary<string, object> parameters = request.ContainsKey("params") ? Object(request["params"]) : null;
            if (method == "initialize") { Console.WriteLine(Response(id, "{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"serverInfo\":{\"name\":\"fake\",\"version\":\"1\"}}")); continue; }
            if (method == "server/discover") { Console.WriteLine(Response(id, "{\"resultType\":\"complete\"}")); continue; }
            if (method == "tools/list")
            {
                bool modernList = Modern(parameters);
                string result = modernList ? "{\"_meta\":" + Meta() + ",\"cacheScope\":\"private\",\"resultType\":\"complete\",\"tools\":" + Tools() + ",\"ttlMs\":0}" : "{\"tools\":" + Tools() + "}";
                Console.WriteLine(Response(id, result));
                continue;
            }
            if (method != "tools/call" || parameters == null || !parameters.ContainsKey("name") || !parameters.ContainsKey("arguments")) { Console.WriteLine(Error(id, -32602, "Invalid tools/call params")); continue; }
            bool invalidKeys = false;
            foreach (string key in parameters.Keys) if (key != "name" && key != "arguments" && key != "_meta") invalidKeys = true;
            if (invalidKeys) { Console.WriteLine(Error(id, -32602, "Invalid tools/call params")); continue; }
            string name = parameters["name"] as string;
            object arguments = parameters["arguments"];
            bool modern = Modern(parameters);
            if (name == "lattice_delivery_run" || name == "lattice_delivery_status")
            {
                if (!DeliveryArguments(arguments)) { MutateRejectedCall(name == "lattice_delivery_run" ? "delivery_run" : "delivery_status"); Console.WriteLine(Error(id, -32602, "Tool accepts no arguments")); continue; }
                Increment(name == "lattice_delivery_run" ? "delivery_run" : "delivery_status", false);
                string code = Environment.GetEnvironmentVariable("TASK038_ACCEPT_FAKE_ERROR_CODE") ?? "LATTICED_DATABASE_CONNECT_REJECTED";
                Console.WriteLine(ToolResult(id, true, ErrorStructured(code), modern));
                continue;
            }
            if (name == "lattice_task_submit")
            {
                string requestId;
                if (!SubmitArguments(arguments, out requestId)) { MutateRejectedCall("task_submit"); Console.WriteLine(Error(id, -32602, "Invalid task submit arguments")); continue; }
                bool effect = !File.Exists(state);
                Increment("task_submit", effect);
                if (effect) File.WriteAllText(state, requestId);
                string stored = File.ReadAllText(state);
                Console.WriteLine(stored == requestId ? ToolResult(id, false, StatusJson, modern) : ToolResult(id, true, ErrorStructured("LATTICE_TASK_REQUEST_SUBSTITUTED"), modern));
                continue;
            }
            if (name == "lattice_task_status")
            {
                string taskRef;
                if (!StatusArguments(arguments, out taskRef)) { MutateRejectedCall("task_status"); Console.WriteLine(Error(id, -32602, "Invalid task status arguments")); continue; }
                Increment("task_status", false);
                Console.WriteLine(taskRef == new string('0', 64) || !File.Exists(state) ? ToolResult(id, true, ErrorStructured("LATTICE_TASK_REFERENCE_REJECTED"), modern) : ToolResult(id, false, StatusJson, modern));
                continue;
            }
            Console.WriteLine(Error(id, -32602, "Unknown tool"));
        }
        return 0;
    }
}
'@

Add-Type -TypeDefinition $source -Language CSharp -ReferencedAssemblies 'System.Web.Extensions.dll' -OutputAssembly $fakeBinary -OutputType ConsoleApplication
$binarySha = (Get-FileHash -LiteralPath $fakeBinary -Algorithm SHA256).Hash.ToLowerInvariant()
$script:SourceRoot = $script:RepoRoot
$script:Psql = $fakeBinary
$PostgresRunId = '0123456789abcdef0123456789abcdef'
$PostgresPort = 49152
$PgCtlExecutable = $fakeBinary
$PostgresDataDirectory = $testRoot
$fakePsqlRejected = $false
try { $null = Get-PostgresDatabaseBinding -Password ('x' * 32) }
catch { $fakePsqlRejected = ([string]$_.Exception.Message -ceq 'TASK038_ACCEPT_POSTGRES_BINDING_REJECTED') }
if (-not $fakePsqlRejected) { throw 'TASK038_ACCEPT_TEST_CALLER_FAKE_PSQL_NOT_REJECTED' }
$commit = (& git -C $script:RepoRoot rev-parse HEAD).Trim().ToLowerInvariant()
if ($LASTEXITCODE -ne 0 -or $commit -notmatch '^[0-9a-f]{40}$') { throw 'TASK038_ACCEPT_TEST_SOURCE_COMMIT_REJECTED' }
$tree = (& git -C $script:RepoRoot rev-parse ($commit + '^{tree}')).Trim().ToLowerInvariant()
if ($LASTEXITCODE -ne 0 -or $tree -notmatch '^[0-9a-f]{40}$') { throw 'TASK038_ACCEPT_TEST_SOURCE_TREE_REJECTED' }

[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_STATE', $fakeState, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_COUNTERS', $fakeCounters, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_SCHEMA_MUTATION', $null, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_TOOL_SET_MUTATION', $null, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_ERROR_CODE', $null, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_ERROR_MUTATION', $null, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_DESCENDANT_ESCAPE', $null, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_DESCENDANT_PID', $descendantPid, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_NEGATIVE_EFFECT', $null, 'Process')
[IO.File]::WriteAllLines($fakeCounters, @(
    'dispatch=0',
    'effect=0',
    'delivery_run=0',
    'delivery_status=0',
    'task_submit=0',
    'task_status=0'
), $script:Utf8)

$positiveEvidence = Join-Path $testRoot 'positive-evidence'
$positive = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'PROTOCOL_ONLY' -EvidenceRoot $positiveEvidence
if ($positive.Classification -cne 'PASS') { throw 'TASK038_ACCEPT_TEST_POSITIVE_REJECTED' }
$positiveFinal = Get-FinalEvidence -EvidenceRoot $positiveEvidence
if (
    [string]$positiveFinal.status -cne 'PASS' -or
    [int]$positiveFinal.exact_pass_marker_count -ne 1 -or
    [bool]$positiveFinal.not_run -or
    [bool]$positiveFinal.delivery_run_invoked -or
    [string]$positiveFinal.tool_schema_contract_sha256 -notmatch '^[0-9a-f]{64}$' -or
    [string]$positiveFinal.tool_error_contract_sha256 -cne $script:ExpectedToolErrorContractSha256 -or
    [string]$positiveFinal.tool_schema_contract_sha256 -ceq [string]$positiveFinal.tool_error_contract_sha256 -or
    -not [bool]$positiveFinal.process_cleanup_checked -or
    [int]$positiveFinal.negative_protocol_case_count -ne 12 -or
    -not [bool]$positiveFinal.negative_protocol_rejections_checked -or
    [bool]$positiveFinal.current_candidate_identity_checked
) { throw 'TASK038_ACCEPT_TEST_POSITIVE_EVIDENCE_REJECTED' }

$caseNames = @($positiveFinal.negative_protocol_cases | ForEach-Object { [string]$_.case } | Sort-Object)
$expectedCases = @(
    '02n01-unknown-tool','02n02-delivery-extra','02n03-delivery-non-object','02n04-submit-unknown-intent',
    '02n05-submit-bad-id','02n06-submit-extra-shell','02n07-submit-extra-sql','02n08-submit-extra-path',
    '02n09-submit-extra-credential','02n10-status-bad','02n11-status-uppercase','02n12-status-extra-ref'
) | Sort-Object
if (@(Compare-Object $expectedCases $caseNames).Count -ne 0) { throw 'TASK038_ACCEPT_TEST_NEGATIVE_CASE_SET_REJECTED' }
foreach ($case in @($positiveFinal.negative_protocol_cases)) {
    if (
        [int]$case.protocol_error_code -ne -32602 -or
        [long]$case.service_dispatch_observed -ne 0 -or
        [long]$case.external_effect_observed -ne 0 -or
        [bool]$case.authoritative_observation -or
        [string]$case.observation_scope -cne 'HARNESS_FAKE_PARSER_ONLY_NOT_LIVE'
    ) {
        throw 'TASK038_ACCEPT_TEST_NEGATIVE_PROTOCOL_REJECTED'
    }
}

$counterValues = @{}
foreach ($line in @(Get-Content -LiteralPath $fakeCounters -Encoding UTF8)) {
    $parts = $line -split '=', 2
    if ($parts.Count -eq 2) { $counterValues[$parts[0]] = [int]$parts[1] }
}
if (
    [int]$counterValues.dispatch -ne 6 -or [int]$counterValues.effect -ne 1 -or
    [int]$counterValues.delivery_run -ne 0 -or [int]$counterValues.delivery_status -ne 1 -or
    [int]$counterValues.task_submit -ne 3 -or [int]$counterValues.task_status -ne 2
) { throw 'TASK038_ACCEPT_TEST_DISPATCH_EFFECT_COUNTER_REJECTED' }

[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_NEGATIVE_EFFECT', '1', 'Process')
$negativeEffectEvidence = Join-Path $testRoot 'negative-protocol-observed-effect'
$negativeEffect = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'PROTOCOL_ONLY' -EvidenceRoot $negativeEffectEvidence
Assert-FixedFailure -Run $negativeEffect -EvidenceRoot $negativeEffectEvidence -FailureCode 'TASK038_ACCEPT_OBSERVED_EFFECT_RECEIPT_REJECTED'
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_NEGATIVE_EFFECT', $null, 'Process')

$allEvidenceText = [string]::Join("`n", @(Get-ChildItem -LiteralPath $positiveEvidence -File | ForEach-Object { Get-Content -LiteralPath $_.FullName -Raw -Encoding UTF8 }))
if ($allEvidenceText -cmatch 'TASK038_NON_SECRET_SENTINEL') { throw 'TASK038_ACCEPT_TEST_ARGUMENT_ECHO_REJECTED' }

$schemaMutations = [ordered]@{
    'delivery-type'='TASK038_ACCEPT_DELIVERY_SCHEMA_REJECTED'; 'delivery-additional'='TASK038_ACCEPT_DELIVERY_SCHEMA_REJECTED'; 'delivery-properties'='TASK038_ACCEPT_DELIVERY_SCHEMA_REJECTED'; 'delivery-output'='TASK038_ACCEPT_DELIVERY_SCHEMA_REJECTED'
    'submit-type'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'submit-required'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'submit-additional'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'submit-client-type'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'submit-client-min'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'submit-client-max'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'submit-client-pattern'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'submit-intent-type'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'submit-intent-enum'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'submit-extra-property'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'
    'status-type'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'status-required'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'status-additional'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'status-ref-type'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'status-ref-min'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'status-ref-max'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'status-ref-pattern'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'; 'status-extra-property'='TASK038_ACCEPT_TASK_SCHEMA_REJECTED'
    'output-type'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-required'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-additional'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-schema-type'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-schema-enum'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-status-type'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-status-enum'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-state-type'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-state-enum'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-task-type'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-task-min'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-task-max'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-task-pattern'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-ledger-type'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-ledger-bounds'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-ledger-pattern'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-result-anyof'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-result-string-type'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-result-bounds'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-result-pattern'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'; 'output-result-null-type'='TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'
}
foreach ($mutation in $schemaMutations.Keys) {
    [Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_SCHEMA_MUTATION', [string]$mutation, 'Process')
    $evidence = Join-Path $testRoot ('schema-' + $mutation)
    $run = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'DISCOVERY_ONLY' -EvidenceRoot $evidence
    Assert-FixedFailure -Run $run -EvidenceRoot $evidence -FailureCode ([string]$schemaMutations[$mutation])
}
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_SCHEMA_MUTATION', $null, 'Process')

foreach ($toolSetMutation in @('two-tools','fifth-tool')) {
    [Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_TOOL_SET_MUTATION', $toolSetMutation, 'Process')
    $evidence = Join-Path $testRoot ('tool-set-' + $toolSetMutation)
    $run = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'DISCOVERY_ONLY' -EvidenceRoot $evidence
    Assert-FixedFailure -Run $run -EvidenceRoot $evidence -FailureCode 'TASK038_ACCEPT_TOOL_DISCOVERY_MISMATCH'
}
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_TOOL_SET_MUTATION', $null, 'Process')

[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_ERROR_CODE', 'LATTICE_PREFIX_MATCHING_BUT_UNKNOWN', 'Process')
$unknownCodeEvidence = Join-Path $testRoot 'unknown-code'
$unknownCode = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'PROTOCOL_ONLY' -EvidenceRoot $unknownCodeEvidence
Assert-FixedFailure -Run $unknownCode -EvidenceRoot $unknownCodeEvidence -FailureCode 'TASK038_ACCEPT_TOOL_CODE_REJECTED'
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_ERROR_CODE', $null, 'Process')

[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_ERROR_MUTATION', 'extra-key', 'Process')
$errorShapeEvidence = Join-Path $testRoot 'error-extra-key'
$errorShape = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'PROTOCOL_ONLY' -EvidenceRoot $errorShapeEvidence
Assert-FixedFailure -Run $errorShape -EvidenceRoot $errorShapeEvidence -FailureCode 'TASK038_ACCEPT_TOOL_RESULT_ENVELOPE_REJECTED'
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_ERROR_MUTATION', $null, 'Process')

[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_DESCENDANT_ESCAPE', '1', 'Process')
$descendantEvidence = Join-Path $testRoot 'descendant-containment'
$descendant = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'DISCOVERY_ONLY' -EvidenceRoot $descendantEvidence
if ($descendant.Classification -cne 'PASS') { throw 'TASK038_ACCEPT_TEST_DESCENDANT_CONTAINMENT_REJECTED' }
$descendantFinal = Get-FinalEvidence -EvidenceRoot $descendantEvidence
$descendantSessions = @($descendantFinal.sessions)
if (
    $descendantSessions.Count -ne 1 -or
    -not [bool]$descendantSessions[0].process_created_suspended -or
    -not [bool]$descendantSessions[0].job_assigned_before_resume -or
    [int]$descendantSessions[0].contained_descendants_terminated -lt 1 -or
    [int]$descendantSessions[0].job_active_processes_after_exit -ne 0
) { throw 'TASK038_ACCEPT_TEST_DESCENDANT_ESCAPE_MUTATION_REJECTED' }
$escapedPid = [int](Get-Content -LiteralPath $descendantPid -Raw -Encoding UTF8)
if ($null -ne (Get-Process -Id $escapedPid -ErrorAction SilentlyContinue)) { throw 'TASK038_ACCEPT_TEST_DESCENDANT_STILL_ACTIVE' }
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_DESCENDANT_ESCAPE', $null, 'Process')

$historicalCommitEvidence = Join-Path $testRoot 'historical-f9-identity'
$historicalCommit = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit 'f9ae267ba3d335aa67bdd9548aadf7218a90c391' -Mode 'FULL' -EvidenceRoot $historicalCommitEvidence
Assert-FixedFailure -Run $historicalCommit -EvidenceRoot $historicalCommitEvidence -FailureCode 'TASK038_ACCEPT_CURRENT_CANDIDATE_COMMIT_REJECTED'

$historicalB4Evidence = Join-Path $testRoot 'historical-b4-identity'
$historicalB4 = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit 'b4cbe19cace38a2b100150d7faf5d695e6e8b685' -Mode 'FULL' -EvidenceRoot $historicalB4Evidence
Assert-FixedFailure -Run $historicalB4 -EvidenceRoot $historicalB4Evidence -FailureCode 'TASK038_ACCEPT_CURRENT_CANDIDATE_COMMIT_REJECTED'

$historical092Evidence = Join-Path $testRoot 'historical-09264024-identity'
$historical092 = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit '0926402400000000000000000000000000000000' -Mode 'FULL' -EvidenceRoot $historical092Evidence
Assert-FixedFailure -Run $historical092 -EvidenceRoot $historical092Evidence -FailureCode 'TASK038_ACCEPT_CURRENT_CANDIDATE_COMMIT_REJECTED'

$preSeedEvidence = Join-Path $testRoot 'pre-clean-seed-ancestry'
$preSeed = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit '512732d5b71a5d373363b77bb23a29e4a8ae3b1b' -Mode 'FULL' -EvidenceRoot $preSeedEvidence
Assert-FixedFailure -Run $preSeed -EvidenceRoot $preSeedEvidence -FailureCode 'TASK038_ACCEPT_CLEAN_SEED_ANCESTRY_REJECTED'

$callerBinaryEvidence = Join-Path $testRoot 'caller-binary-cannot-prove-candidate'
$callerBinary = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'FULL' -EvidenceRoot $callerBinaryEvidence -IncludeCallerBinary -AdditionalArguments @{
    ExpectedSourceTree = $tree
    ExpectedToolSchemaContractSha256 = [string]$positiveFinal.tool_schema_contract_sha256
    ExpectedToolErrorContractSha256 = [string]$positiveFinal.tool_error_contract_sha256
    CurrentCandidateReviewCommitment = ('1' * 64)
    CurrentCandidateAcceptanceCommitment = ('2' * 64)
}
Assert-FixedFailure -Run $callerBinary -EvidenceRoot $callerBinaryEvidence -FailureCode 'TASK038_ACCEPT_CALLER_BINARY_TRUST_REJECTED'

$arbitraryCommitmentEvidence = Join-Path $testRoot 'caller-commitments-forbidden'
$arbitraryCommitment = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'FULL' -EvidenceRoot $arbitraryCommitmentEvidence -AdditionalArguments @{
    ExpectedSourceTree = $tree
    ExpectedToolSchemaContractSha256 = [string]$positiveFinal.tool_schema_contract_sha256
    ExpectedToolErrorContractSha256 = [string]$positiveFinal.tool_error_contract_sha256
    CurrentCandidateReviewCommitment = ('1' * 64)
    CurrentCandidateAcceptanceCommitment = ('2' * 64)
}
Assert-FixedFailure -Run $arbitraryCommitment -EvidenceRoot $arbitraryCommitmentEvidence -FailureCode 'TASK038_ACCEPT_CALLER_COMMITMENT_REJECTED'

$treeMutationEvidence = Join-Path $testRoot 'candidate-tree-mutation'
$treeMutation = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'FULL' -EvidenceRoot $treeMutationEvidence -AdditionalArguments @{
    RequirePostgresRestart = [Management.Automation.SwitchParameter]::Present
    ExpectedSourceTree = '0000000000000000000000000000000000000000'
    ExpectedToolSchemaContractSha256 = [string]$positiveFinal.tool_schema_contract_sha256
    ExpectedToolErrorContractSha256 = [string]$positiveFinal.tool_error_contract_sha256
}
Assert-FixedFailure -Run $treeMutation -EvidenceRoot $treeMutationEvidence -FailureCode 'TASK038_ACCEPT_CURRENT_CANDIDATE_TREE_REJECTED'

$restartMissingEvidence = Join-Path $testRoot 'full-restart-is-mandatory'
$restartMissing = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'FULL' -EvidenceRoot $restartMissingEvidence -AdditionalArguments @{
    ExpectedSourceTree = $tree
    ExpectedToolSchemaContractSha256 = [string]$positiveFinal.tool_schema_contract_sha256
    ExpectedToolErrorContractSha256 = [string]$positiveFinal.tool_error_contract_sha256
}
Assert-FixedFailure -Run $restartMissing -EvidenceRoot $restartMissingEvidence -FailureCode 'TASK038_ACCEPT_POSTGRES_RESTART_REQUIRED'

$replayedReceiptEvidence = Join-Path $testRoot 'candidate-caller-commitment-replay'
$replayedReceipt = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'FULL' -EvidenceRoot $replayedReceiptEvidence -AdditionalArguments @{
    ExpectedSourceTree = $tree
    ExpectedToolSchemaContractSha256 = [string]$positiveFinal.tool_schema_contract_sha256
    ExpectedToolErrorContractSha256 = [string]$positiveFinal.tool_error_contract_sha256
    CurrentCandidateReviewCommitment = ('1' * 64)
    CurrentCandidateAcceptanceCommitment = ('1' * 64)
}
Assert-FixedFailure -Run $replayedReceipt -EvidenceRoot $replayedReceiptEvidence -FailureCode 'TASK038_ACCEPT_CALLER_COMMITMENT_REJECTED'

$lifecycleMissingEvidence = Join-Path $testRoot 'tunnel-lifecycle-not-materialized'
$lifecycleMissing = Invoke-Runner -Binary $fakeBinary -BinarySha $binarySha -Commit $commit -Mode 'FULL' -EvidenceRoot $lifecycleMissingEvidence -AdditionalArguments @{
    RequirePostgresRestart = [Management.Automation.SwitchParameter]::Present
    ExpectedSourceTree = $tree
    ExpectedToolSchemaContractSha256 = [string]$positiveFinal.tool_schema_contract_sha256
    ExpectedToolErrorContractSha256 = [string]$positiveFinal.tool_error_contract_sha256
}
Assert-FixedFailure -Run $lifecycleMissing -EvidenceRoot $lifecycleMissingEvidence -FailureCode 'TASK038_ACCEPT_TUNNEL_LIFECYCLE_RECEIPT_REQUIRED'

[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_STATE', $null, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_COUNTERS', $null, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_DESCENDANT_PID', $null, 'Process')
[Environment]::SetEnvironmentVariable('TASK038_ACCEPT_FAKE_NEGATIVE_EFFECT', $null, 'Process')
Write-Output 'TASK038_FOUR_TOOL_ACCEPTANCE_TEST=PASS'
Write-Output ('TEST_EVIDENCE_ROOT=' + $testRoot)
