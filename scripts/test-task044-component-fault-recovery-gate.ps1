[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$runnerPath = Join-Path $PSScriptRoot 'run-task044-component-fault-recovery-gate.ps1'
if (-not (Test-Path -LiteralPath $runnerPath -PathType Leaf)) {
    throw 'TASK044_RUNNER_MISSING'
}

$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    $runnerPath,
    [ref]$tokens,
    [ref]$parseErrors
)
if ($parseErrors.Count -ne 0) {
    throw 'TASK044_RUNNER_PARSE_REJECTED'
}

$source = [IO.File]::ReadAllText($runnerPath)
$requiredShapes = @(
    "schema_version = 'lattice.task044.component-fault-recovery.v1'",
    "'F01', 'F02', 'F03', 'F04', 'F05', 'F06', 'F07', 'F08'",
    "TASK044_COMPONENT_FAULT_RECOVERY_GATE=PASS",
    "FRESH_STATUS_ZERO_RERUN=PASS",
    "CLEANUP_FOOTPRINT=PASS",
    "TASK044_AGGREGATE_RUNNER_SELF_TEST=PASS",
    "test-task050-autonomy-receipt-acceptance.ps1",
    "-RunTask076WriterLeaseGate",
    "-RunTask044CancellationGate",
    "TASK044_POSTGRES_CANCEL_REPLAY=PASS",
    "cargo test",
    "git status --short"
)
foreach ($shape in $requiredShapes) {
    if ($source.IndexOf($shape, [StringComparison]::Ordinal) -lt 0) {
        throw 'TASK044_RUNNER_SHAPE_REJECTED'
    }
}

$forbiddenShapes = @(
    'Invoke-Expression',
    'Start-Process',
    'Remove-Item -Recurse',
    'TASK044_COMPONENT_FAULT_RECOVERY_GATE=PASS`n',
    'LATTICE_TASK019_PASSWORD='
)
foreach ($shape in $forbiddenShapes) {
    if ($source.IndexOf($shape, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
        throw 'TASK044_RUNNER_UNSAFE_SHAPE_REJECTED'
    }
}

$selfTestOutput = & powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $runnerPath -SelfTestOnly 2>&1
if ($LASTEXITCODE -ne 0) {
    throw 'TASK044_RUNNER_SELF_TEST_REJECTED'
}
$selfTestLines = @($selfTestOutput | ForEach-Object { [string]$_ })
if (@($selfTestLines | Where-Object { $_ -ceq 'TASK044_AGGREGATE_RUNNER_SELF_TEST=PASS' }).Count -ne 1) {
    throw 'TASK044_RUNNER_SELF_TEST_REJECTED'
}
if (@($selfTestLines | Where-Object { $_ -ceq 'TASK044_COMPONENT_FAULT_RECOVERY_GATE=PASS' }).Count -ne 0) {
    throw 'TASK044_SELF_TEST_MUST_NOT_EMIT_GATE_PASS'
}

git -C $repositoryRoot diff --check -- scripts/test-task044-component-fault-recovery-gate.ps1
if ($LASTEXITCODE -ne 0) {
    throw 'TASK044_TEST_DIFF_REJECTED'
}

Write-Output 'TASK044_COMPONENT_FAULT_RECOVERY_SELF_TEST=PASS'
