[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$harness = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'run-task037-full-chain-verification.ps1'))
$source = [IO.File]::ReadAllText($harness, [Text.Encoding]::UTF8)

foreach ($forbidden in @(
    'taskkill.exe',
    'Stop-OwnedProcessTree',
    'Publish-Task037SanitizedLogEvidence',
    'Redact-Task037LogText',
    'IsInAnyJob',
    'CreateKillOnCloseNamed',
    "-Filter '*.credential-owner.json'",
    "-Filter '*.started.json'"
)) {
    if ($source.IndexOf($forbidden, [StringComparison]::Ordinal) -ge 0) {
        throw ('TASK037_CONTAINMENT_FORBIDDEN_SOURCE|' + $forbidden)
    }
}

foreach ($required in @(
    '-EnvironmentValues $launchEnvironment',
    '-Executable $PowerShellExecutable',
    '[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)',
    'Get-Task037RawArtifactReceipt',
    'content_retained = $false',
    'raw_log_root_removed = $rawLogRootRemoved',
    'OwnerRecordLock = $recordLock',
    "-Candidate ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_CONTROLLER_NODE_EXE', 'Process'))",
    '-ProgramDataAuthority $programDataAuthority',
    'Assert-Task037ProcessInJob -Job $Job -Process $childProcess',
    'Stop-Task037Job -Job $phaseJob',
    'job_active_process_count_zero = $jobProcessCountZero',
    "Join-Path `$evidenceRoot 'final.pending.json'",
    'credential_source_unchanged = $credentialSourceUnchanged',
    "throw 'TASK037_ACCEPTANCE_FINALIZATION_REJECTED'",
    'Promote-Task037FinalEvidence -PendingPath $pendingFinalPath -FinalPath $finalPath',
    'New-Task037CleanupSupervision',
    'Complete-Task037CleanupSupervision',
    'Invoke-Task037RecoveryOnly',
    'Set-Task037RestrictedDirectoryAcl',
    'Get-Task037RestrictedAclEvidence',
    "`$watcher.Filter = 'auth.json'",
    "[switch]`$InternalController",
    "throw 'TASK037_UNSUPERVISED_CONTROLLER_REJECTED'",
    'Assert-Task037SupervisorBinding',
    'Assert-Task037CurrentProcessInNamedJob',
    'OpenForQuery',
    'CreateQueryOnlyAttestationNamed',
    'VerifyEffectOpenDenied',
    '(A;;0x00000004;;;OW)',
    '-AttestationJob $supervision.AttestationJob',
    'TASK037_SELF_TEST_ATTESTATION_HANDLE_ISOLATION_REJECTED',
    'LATTICE_TASK037_SUPERVISOR_OWNER_TOKEN',
    'LATTICE_TASK037_CONTROLLER_CARGO_EXE',
    'LATTICE_TASK037_CONTROLLER_GIT_EXE',
    'LATTICE_TASK037_CONTROLLER_NODE_EXE',
    'LATTICE_TASK037_CONTROLLER_RUSTC_EXE',
    'LATTICE_TASK037_CONTROLLER_RUSTDOC_EXE',
    'Get-TrustedTask037RustToolchain',
    'Assert-Task037RustToolchainVersion',
    'Assert-Task037CargoConfigurationClosure',
    "RUSTUP_TOOLCHAIN = `$task037RustToolchain",
    "CARGO_NET_OFFLINE = 'true'",
    'GIT_CONFIG_NOSYSTEM',
    'Start-Task037CredentialSourceMonitor',
    'Write-Task037ValidatedPhaseEvidence',
    'Write-Task037AdmissionStageReceipt',
    'Read-Task037AdmissionStageReceipt',
    "[switch]`$InternalAdmissionProbe",
    "[switch]`$HarnessControllerPreflightProbe",
    "[switch]`$InternalControllerPreflightOnly",
    "[switch]`$HarnessPublicFailureProbe",
    'LATTICE_TASK037_ADMISSION_STARTED_AT_UTC',
    "'recovery_preflight'",
    "'supervision_create'",
    "'gated_controller_start'",
    "'supervisor_binding'",
    "'git_identity_preflight'",
    "'node_identity_preflight'",
    "'node_runtime_preflight'",
    "'rust_toolchain_preflight'",
    "'artifact_identity_preflight'",
    "'worktree_preflight'",
    "'inner_pre_acceptance'",
    'lattice.task037.admission-stage.v2',
    'ALLUSERSPROFILE = $programData',
    'LATTICE_TASK037_CONTROLLER_PROGRAM_DATA',
    '-TrustedProgramData $programDataAuthority',
    'node-identity-probe.release',
    '[AllowEmptyCollection()][string[]]$Arguments',
    'if ($arguments.Count -gt 0)',
    "PSModuleAnalysisCachePath = 'NUL'",
    '$payloadNames.Contains([string]$name)',
    'Get-Task037ClosedGitSnapshot',
    "'InitializeSynthetic'",
    "'AddSynthetic'",
    "'CommitSynthetic'",
    "GIT_OPTIONAL_LOCKS = '0'",
    "GIT_NO_REPLACE_OBJECTS = '1'",
    "'core.fsmonitor=false'",
    "'core.excludesFile=NUL'",
    'TASK037_POST_RUN_WORKTREE_IDENTITY_REJECTED',
    'TASK037_POST_RUN_WORKTREE_DIRTY_REJECTED',
    'TASK037_SELF_TEST_CLOSED_GIT_REJECTED',
    'TASK037_SELF_TEST_EMPTY_ARGUMENTS_REJECTED',
    'TASK037_CONTROLLER_PREFLIGHT=PASS',
    'TASK037_SYNTHETIC_ADMISSION_REJECTED',
    'TASK037_SELF_TEST_ADMISSION_RECEIPT_REJECTED',
    'Get-Task037RepositoryDirectoryShape',
    'Assert-Task037RepositoryDirectoryShapeUnchanged',
    'TASK037_POST_RUN_DIRECTORY_SHAPE_REJECTED',
    'TASK037_CHILD_SPECIAL_FOLDER_REJECTED',
    'TASK037_MCP_RESPONSE_JSON_REJECTED',
    'TASK037_SELF_TEST_PROFILE_OVERRIDE_REJECTED',
    'lattice.task037.cleanup.v2',
    'failure_gate = $failureGate',
    'last_completed_gate = $lastCompletedGate',
    'Invoke-Task037ProcessContractCheck',
    "'process_exit_snapshot'",
    "'wrapper_reap'",
    "'wrapper_exit_match'",
    "'process_handle_release'",
    "'openclaw_liveness'",
    "'job_cardinality'",
    "'child_exit_success'",
    "'phase_meta_projection'",
    "'success_tool_result'",
    "'success_tool_error_configuration'",
    "'success_tool_error_delivery'",
    "'success_tool_error_graph'",
    "'success_tool_error_hermes'",
    "'success_tool_error_transport'",
    "'success_tool_error_unclassified'",
    "'success_launcher_binding'",
    "'success_delivery_contract'",
    "'success_graph_contract'",
    "'success_hermes_contract'",
    "'success_digest_contract'",
    "'success_finding_items'",
    "'success_action_items'",
    "'success_answer_bytes'",
    'Get-Task037ToolFailureGate',
    'Test-Task037JsonInteger',
    'Test-Task037JsonArray',
    '$result.content -isnot [object[]]',
    'TASK037_SELF_TEST_TOOL_CONTENT_TYPE_REJECTED',
    'TASK037_SELF_TEST_SUCCESS_SEMANTICS_REJECTED',
    'Release-Task037ExitedProcessHandles',
    'Wait-Task037JobProcessBaseline',
    'Get-Task037JobProcessCountSnapshot',
    'Get-Task037CoherentJobProcessSnapshot',
    'Get-Task037OpenClawJobBaseline',
    'Test-Task037ProcessIdSetSubset',
    'Test-Task037ProcessIdentityCohortLiveInJob',
    'ProcessStartTicks',
    'RequiredProcessIdentities',
    '[LatticeTask037JobObjectInterop]::ProcessCounts',
    '[LatticeTask037JobObjectInterop]::ProcessIds',
    'JobObjectBasicProcessIdList = 3',
    'process_exit_code = [int]$processContract.ChildExitCode',
    'launcher_path_sha256',
    "'TASK037_LATTICE_COMPONENT_FAILURE'"
)) {
    if ($source.IndexOf($required, [StringComparison]::Ordinal) -lt 0) {
        throw ('TASK037_CONTAINMENT_REQUIRED_SOURCE_MISSING|' + $required)
    }
}
$processContractStart = $source.LastIndexOf(
    '$processContract = [pscustomobject]@{',
    [StringComparison]::Ordinal
)
$processContractCursor = $processContractStart
foreach ($fragment in @(
    "-Gate 'process_exit_snapshot'",
    "-Gate 'wrapper_reap'",
    "-Gate 'wrapper_exit_match'",
    "-Gate 'openclaw_liveness'",
    "-Gate 'child_exit_success'",
    "-Gate 'process_handle_release'",
    "-Gate 'job_cardinality'",
    "-Gate 'raw_bounds'",
    'process_exit_code = [int]$processContract.ChildExitCode'
)) {
    if ($processContractCursor -lt 0) {
        throw 'TASK037_CONTAINMENT_PROCESS_CONTRACT_ORDER_REJECTED'
    }
    $processContractCursor = $source.IndexOf(
        $fragment,
        $processContractCursor,
        [StringComparison]::Ordinal
    )
    if ($processContractCursor -lt 0) {
        throw 'TASK037_CONTAINMENT_PROCESS_CONTRACT_ORDER_REJECTED'
    }
    $processContractCursor += $fragment.Length
}
if ($source.IndexOf('Start-Process -FilePath $fullChainExe', [StringComparison]::Ordinal) -ge 0) {
    throw 'TASK037_CONTAINMENT_DIRECT_FULL_CHAIN_START_REJECTED'
}

$successCallStart = $source.LastIndexOf(
    '$toolCodeValue = if (',
    [StringComparison]::Ordinal
)
$successCallCursor = $successCallStart
foreach ($fragment in @(
    "-Gate 'phase_meta_projection'",
    'Get-Task037ToolFailureGate -Code $toolCodeValue',
    '-Gate $toolFailureGate',
    "throw ('TASK037_' + `$Phase.ToUpperInvariant() + '_TOOL_ERROR')",
    "-Gate 'success_tool_result'",
    'Assert-FullChainSuccess',
    "-Gate 'memory_expected'"
)) {
    if ($successCallCursor -lt 0) {
        throw 'TASK037_CONTAINMENT_SUCCESS_CALL_ORDER_REJECTED'
    }
    $successCallCursor = $source.IndexOf(
        $fragment,
        $successCallCursor,
        [StringComparison]::Ordinal
    )
    if ($successCallCursor -lt 0) {
        throw 'TASK037_CONTAINMENT_SUCCESS_CALL_ORDER_REJECTED'
    }
    $successCallCursor += $fragment.Length
}

$successFunctionStart = $source.IndexOf(
    'function Assert-FullChainSuccess {',
    [StringComparison]::Ordinal
)
$successFunctionCursor = $successFunctionStart
foreach ($fragment in @(
    "-Gate 'success_launcher_binding'",
    "-Gate 'success_delivery_contract'",
    "-Gate 'success_graph_contract'",
    "-Gate 'success_hermes_contract'",
    "-Gate 'success_digest_contract'",
    "-Gate 'success_finding_items'",
    "-Gate 'success_action_items'",
    "-Gate 'success_answer_bytes'"
)) {
    if ($successFunctionCursor -lt 0) {
        throw 'TASK037_CONTAINMENT_SUCCESS_GATE_ORDER_REJECTED'
    }
    $successFunctionCursor = $source.IndexOf(
        $fragment,
        $successFunctionCursor,
        [StringComparison]::Ordinal
    )
    if ($successFunctionCursor -lt 0) {
        throw 'TASK037_CONTAINMENT_SUCCESS_GATE_ORDER_REJECTED'
    }
    $successFunctionCursor += $fragment.Length
}
if ($source.IndexOf("Get-Command 'cargo.exe'", [StringComparison]::Ordinal) -ge 0) {
    throw 'TASK037_CONTAINMENT_AMBIENT_CARGO_LOOKUP_REJECTED'
}

$powerShell = Join-Path $PSHOME 'powershell.exe'
$bindingNames = @(
    'LATTICE_TASK037_SUPERVISOR_ACCEPTANCE_ID',
    'LATTICE_TASK037_SUPERVISOR_MANIFEST_PATH',
    'LATTICE_TASK037_SUPERVISOR_OWNER_TOKEN',
    'LATTICE_TASK037_SUPERVISOR_TEMP_BASE',
    'LATTICE_TASK037_SUPERVISOR_TEMP_ROOT'
)
$bindingOriginal = @{}
foreach ($name in $bindingNames) {
    $bindingOriginal[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    [Environment]::SetEnvironmentVariable($name, $null, 'Process')
}
$directProcess = $null
try {
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $powerShell
    $startInfo.Arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $harness + '" -InternalController'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $directProcess = [Diagnostics.Process]::new()
    $directProcess.StartInfo = $startInfo
    if (-not $directProcess.Start()) { throw 'TASK037_DIRECT_INTERNAL_BINDING_PROBE_REJECTED' }
    $directStdout = $directProcess.StandardOutput.ReadToEndAsync()
    $directStderr = $directProcess.StandardError.ReadToEndAsync()
    if (-not $directProcess.WaitForExit(10000)) {
        $directProcess.Kill()
        $directProcess.WaitForExit()
        throw 'TASK037_DIRECT_INTERNAL_BINDING_PROBE_REJECTED'
    }
    $directText = $directStdout.GetAwaiter().GetResult() + $directStderr.GetAwaiter().GetResult()
    if (
        $directProcess.ExitCode -eq 0 -or
        $directText.IndexOf('TASK037_CONTROLLER_ACCEPTANCE_ID_REJECTED', [StringComparison]::Ordinal) -lt 0
    ) {
        throw 'TASK037_DIRECT_INTERNAL_BINDING_PROBE_REJECTED'
    }
}
finally {
    if ($null -ne $directProcess) { $directProcess.Dispose() }
    foreach ($name in $bindingNames) {
        [Environment]::SetEnvironmentVariable($name, $bindingOriginal[$name], 'Process')
    }
}

$phaseProbe = $null
$phaseProbeRoot = Join-Path ([IO.Path]::GetTempPath()) (
    'lattice-task037-direct-phase-' + [Guid]::NewGuid().ToString('N')
)
try {
    foreach ($directory in @(
        $phaseProbeRoot,
        (Join-Path $phaseProbeRoot 'home'),
        (Join-Path $phaseProbeRoot 'home\AppData'),
        (Join-Path $phaseProbeRoot 'home\AppData\Roaming'),
        (Join-Path $phaseProbeRoot 'home\AppData\Local'),
        (Join-Path $phaseProbeRoot 'temp')
    )) {
        New-Item -ItemType Directory -Path $directory -Force:$false | Out-Null
    }
    $phaseAcceptanceId = 'f' * 32
    $phaseInfo = [Diagnostics.ProcessStartInfo]::new()
    $phaseInfo.FileName = $powerShell
    $phaseInfo.Arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $harness + '" -InternalPhase FullChainPreStatus'
    $phaseInfo.WorkingDirectory = $phaseProbeRoot
    $phaseInfo.UseShellExecute = $false
    $phaseInfo.CreateNoWindow = $true
    $phaseInfo.RedirectStandardOutput = $true
    $phaseInfo.RedirectStandardError = $true
    $phaseInfo.EnvironmentVariables.Clear()
    foreach ($entry in ([ordered]@{
        SystemRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)
        WINDIR = [Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)
        TEMP = Join-Path $phaseProbeRoot 'temp'
        TMP = Join-Path $phaseProbeRoot 'temp'
        USERPROFILE = Join-Path $phaseProbeRoot 'home'
        HOME = Join-Path $phaseProbeRoot 'home'
        APPDATA = Join-Path $phaseProbeRoot 'home\AppData\Roaming'
        LOCALAPPDATA = Join-Path $phaseProbeRoot 'home\AppData\Local'
        PSModuleAnalysisCachePath = 'NUL'
        LATTICE_TASK037_ACCEPTANCE_ID = $phaseAcceptanceId
        LATTICE_TASK037_TEMP_ACCEPTANCE_ROOT = Join-Path ([IO.Path]::GetTempPath()) ('lattice-task037-' + $phaseAcceptanceId)
        LATTICE_TASK019_RUN_ID = 'e' * 32
        LATTICE_TASK019_HOST = '127.0.0.1'
        LATTICE_TASK019_PORT = '55432'
        LATTICE_TASK019_PASSWORD = 'synthetic-not-a-secret'
        LATTICE_TASK019_LIVE = '1'
        LATTICE_TASK019_PHASE = 'restart'
    }).GetEnumerator()) {
        $phaseInfo.EnvironmentVariables[[string]$entry.Key] = [string]$entry.Value
    }
    $phaseProbe = [Diagnostics.Process]::new()
    $phaseProbe.StartInfo = $phaseInfo
    if (-not $phaseProbe.Start()) { throw 'TASK037_DIRECT_PHASE_BINDING_PROBE_REJECTED' }
    $phaseStdout = $phaseProbe.StandardOutput.ReadToEndAsync()
    $phaseStderr = $phaseProbe.StandardError.ReadToEndAsync()
    if (-not $phaseProbe.WaitForExit(10000)) {
        $phaseProbe.Kill()
        $phaseProbe.WaitForExit()
        throw 'TASK037_DIRECT_PHASE_BINDING_PROBE_REJECTED'
    }
    $phaseText = $phaseStdout.GetAwaiter().GetResult() + $phaseStderr.GetAwaiter().GetResult()
    if (
        $phaseProbe.ExitCode -eq 0 -or
        $phaseText.IndexOf('TASK037_SUPERVISOR_BINDING_TEMP_BASE_REJECTED', [StringComparison]::Ordinal) -lt 0
    ) {
        throw 'TASK037_DIRECT_PHASE_BINDING_PROBE_REJECTED'
    }
}
finally {
    if ($null -ne $phaseProbe) { $phaseProbe.Dispose() }
    if (Test-Path -LiteralPath $phaseProbeRoot) {
        if (Test-Path -LiteralPath (Join-Path $phaseProbeRoot 'Microsoft')) {
            throw 'TASK037_DIRECT_PHASE_SPECIAL_FOLDER_REJECTED'
        }
        $phaseProbeItems = @(Get-ChildItem -LiteralPath $phaseProbeRoot -Recurse -Force)
        if (@($phaseProbeItems | Where-Object {
            $_.Attributes -band [IO.FileAttributes]::ReparsePoint
        }).Count -ne 0) {
            throw 'TASK037_DIRECT_PHASE_BINDING_PROBE_REPARSE_REJECTED'
        }
        [IO.Directory]::Delete($phaseProbeRoot, $true)
    }
}

$publicFailureProbe = $null
try {
    $publicFailureInfo = [Diagnostics.ProcessStartInfo]::new()
    $publicFailureInfo.FileName = $powerShell
    $publicFailureInfo.Arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $harness + '" -HarnessPublicFailureProbe'
    $publicFailureInfo.UseShellExecute = $false
    $publicFailureInfo.CreateNoWindow = $true
    $publicFailureInfo.RedirectStandardOutput = $true
    $publicFailureInfo.RedirectStandardError = $true
    $publicFailureProbe = [Diagnostics.Process]::new()
    $publicFailureProbe.StartInfo = $publicFailureInfo
    if (-not $publicFailureProbe.Start()) { throw 'TASK037_PUBLIC_FAILURE_BOUNDARY_PROBE_REJECTED' }
    $publicFailureStdout = $publicFailureProbe.StandardOutput.ReadToEndAsync()
    $publicFailureStderr = $publicFailureProbe.StandardError.ReadToEndAsync()
    if (-not $publicFailureProbe.WaitForExit(10000)) {
        $publicFailureProbe.Kill()
        $publicFailureProbe.WaitForExit()
        throw 'TASK037_PUBLIC_FAILURE_BOUNDARY_PROBE_REJECTED'
    }
    $publicFailureText = $publicFailureStdout.GetAwaiter().GetResult() + $publicFailureStderr.GetAwaiter().GetResult()
    if (
        $publicFailureProbe.ExitCode -eq 0 -or
        $publicFailureText.IndexOf('TASK037_UNCLASSIFIED_FAILURE', [StringComparison]::Ordinal) -lt 0 -or
        $publicFailureText.IndexOf('task037-public-failure-probe-payload', [StringComparison]::Ordinal) -ge 0
    ) {
        throw 'TASK037_PUBLIC_FAILURE_BOUNDARY_PROBE_REJECTED'
    }
}
finally {
    if ($null -ne $publicFailureProbe) { $publicFailureProbe.Dispose() }
}

$output = @(
    & $powerShell `
        -NoLogo `
        -NoProfile `
        -NonInteractive `
        -ExecutionPolicy Bypass `
        -File $harness `
        -HarnessSelfTest 2>&1
)
if (
    $LASTEXITCODE -ne 0 -or
    $output.Count -ne 1 -or
    [string]$output[0] -ne 'TASK037_VERIFIER_CONTAINMENT_SELF_TEST=PASS'
) {
    throw ('TASK037_CONTAINMENT_SELF_TEST_REJECTED|' + $LASTEXITCODE)
}

$preflightOutput = @(
    & $powerShell `
        -NoLogo `
        -NoProfile `
        -NonInteractive `
        -ExecutionPolicy Bypass `
        -File $harness `
        -HarnessControllerPreflightProbe 2>&1
)
if (
    $LASTEXITCODE -ne 0 -or
    $preflightOutput.Count -ne 1 -or
    [string]$preflightOutput[0] -ne 'TASK037_CONTROLLER_PREFLIGHT=PASS'
) {
    throw ('TASK037_CONTROLLER_PREFLIGHT_PROBE_REJECTED|' + $LASTEXITCODE)
}

Write-Output 'TASK037_VERIFIER_CONTAINMENT_TEST=PASS'
