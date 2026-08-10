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
    "Get-TrustedTask037Node -Candidate ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_CONTROLLER_NODE_EXE', 'Process'))",
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
    "[switch]`$HarnessPublicFailureProbe",
    'LATTICE_TASK037_ADMISSION_STARTED_AT_UTC',
    "'recovery_preflight'",
    "'supervision_create'",
    "'gated_controller_start'",
    "'supervisor_binding'",
    "'inner_pre_acceptance'",
    'TASK037_SYNTHETIC_ADMISSION_REJECTED',
    'TASK037_SELF_TEST_ADMISSION_RECEIPT_REJECTED',
    'launcher_path_sha256',
    "'TASK037_LATTICE_COMPONENT_FAILURE'"
)) {
    if ($source.IndexOf($required, [StringComparison]::Ordinal) -lt 0) {
        throw ('TASK037_CONTAINMENT_REQUIRED_SOURCE_MISSING|' + $required)
    }
}
if ($source.IndexOf('Start-Process -FilePath $fullChainExe', [StringComparison]::Ordinal) -ge 0) {
    throw 'TASK037_CONTAINMENT_DIRECT_FULL_CHAIN_START_REJECTED'
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
try {
    $phaseAcceptanceId = 'f' * 32
    $phaseInfo = [Diagnostics.ProcessStartInfo]::new()
    $phaseInfo.FileName = $powerShell
    $phaseInfo.Arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $harness + '" -InternalPhase FullChainPreStatus'
    $phaseInfo.UseShellExecute = $false
    $phaseInfo.CreateNoWindow = $true
    $phaseInfo.RedirectStandardOutput = $true
    $phaseInfo.RedirectStandardError = $true
    $phaseInfo.EnvironmentVariables.Clear()
    foreach ($entry in ([ordered]@{
        SystemRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)
        WINDIR = [Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)
        TEMP = [IO.Path]::GetTempPath()
        TMP = [IO.Path]::GetTempPath()
        USERPROFILE = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
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

Write-Output 'TASK037_VERIFIER_CONTAINMENT_TEST=PASS'
