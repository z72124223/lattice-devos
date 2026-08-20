[CmdletBinding()]
param(
    [switch]$SelfTestOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$runnerPath = Join-Path $PSScriptRoot 'run-task050-autonomy-receipt-acceptance.ps1'

function Assert-Task050EmbeddedHarnessUsesStoreOnly {
    $tokens = $null
    $parseErrors = $null
    $runnerAst = [Management.Automation.Language.Parser]::ParseFile(
        $runnerPath,
        [ref]$tokens,
        [ref]$parseErrors
    )
    if (@($parseErrors).Count -ne 0) {
        throw 'TASK050_RUNNER_PARSE_REJECTED'
    }

    $embeddedInvocations = @($runnerAst.FindAll({
        param($node)
        $node -is [Management.Automation.Language.CommandAst] -and
        $node.CommandElements.Count -gt 0 -and
        $node.CommandElements[0] -is [Management.Automation.Language.VariableExpressionAst] -and
        $node.CommandElements[0].VariablePath.UserPath -ceq 'harness'
    }, $true))
    if (
        $embeddedInvocations.Count -ne 1 -or
        $embeddedInvocations[0].InvocationOperator -ne [Management.Automation.Language.TokenKind]::Ampersand -or
        $embeddedInvocations[0].CommandElements.Count -ne 2 -or
        $embeddedInvocations[0].CommandElements[1] -isnot [Management.Automation.Language.CommandParameterAst] -or
        $embeddedInvocations[0].CommandElements[1].ParameterName -cne 'StoreOnly'
    ) {
        throw 'TASK050_EMBEDDED_HARNESS_STORE_ONLY_REJECTED'
    }

    $runnerSource = [Text.UTF8Encoding]::new($false, $true).GetString(
        [IO.File]::ReadAllBytes($runnerPath)
    )
    foreach ($forbidden in @('& $harness -MemoryOnly', '& $harness -RunTask075MemoryGate')) {
        if ($runnerSource.Contains($forbidden)) {
            throw 'TASK050_EMBEDDED_HARNESS_PROFILE_REJECTED'
        }
    }
}

function Assert-Task050CanonicalProfileSelectorIsPrivateAndClosed {
    $runnerSource = [Text.UTF8Encoding]::new($false, $true).GetString(
        [IO.File]::ReadAllBytes($runnerPath)
    )
    foreach ($required in @(
        'LATTICE_TASK050_ACCEPTANCE_PROFILE',
        'LATTICE_TASK050_ACCEPTANCE_TASK_SPEC_SHA256',
        'database_run_id',
        'task_spec_digest',
        'task050_canonical_latticed_profiles_when_provisioned'
    )) {
        if (-not $runnerSource.Contains($required)) {
            throw 'TASK050_CANONICAL_PROFILE_SELECTOR_SHAPE_REJECTED'
        }
    }
}

function Assert-Task050ProceedWriterMatrixShape {
    $tokens = $null
    $parseErrors = $null
    $runnerAst = [Management.Automation.Language.Parser]::ParseFile(
        $runnerPath,
        [ref]$tokens,
        [ref]$parseErrors
    )
    if (@($parseErrors).Count -ne 0) {
        throw 'TASK050_PROCEED_WRITER_MATRIX_SHAPE_REJECTED'
    }
    $matrixFunctions = @($runnerAst.FindAll({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -ceq 'Invoke-Task050ProceedWriterMatrix'
    }, $true))
    $matrixStartInfoFunctions = @($runnerAst.FindAll({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -ceq 'New-Task050ProceedWriterMatrixProcessStartInfo'
    }, $true))
    $matrixAuthorityFunctions = @($runnerAst.FindAll({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -ceq 'Get-Task050ProceedWriterMatrixAuthorityProfile'
    }, $true))
    $matrixOutputFunctions = @($runnerAst.FindAll({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -ceq 'Assert-Task050ProceedWriterMatrixOutput'
    }, $true))
    $runnerSource = [Text.UTF8Encoding]::new($false, $true).GetString(
        [IO.File]::ReadAllBytes($runnerPath)
    )
    $matrixSource = if (
        $matrixFunctions.Count -eq 1 -and
        $matrixStartInfoFunctions.Count -eq 1 -and
        $matrixAuthorityFunctions.Count -eq 1 -and
        $matrixOutputFunctions.Count -eq 1
    ) {
        @(
            $matrixAuthorityFunctions[0].Extent.Text,
            $matrixStartInfoFunctions[0].Extent.Text,
            $matrixOutputFunctions[0].Extent.Text,
            $matrixFunctions[0].Extent.Text
        ) -join "`n"
    }
    else {
        ''
    }
    foreach ($required in @(
        'New-Task050ProceedWriterMatrixProcessStartInfo',
        'Get-Task050ProceedWriterMatrixAuthorityProfile',
        'Assert-Task050ProceedWriterMatrixOutput',
        "Get-Task050PhaseProfileInputs -Phase 'restart' -TestOutput `$TestOutput",
        "'test', '--locked', '-p', 'lattice-runtime', '--test', 'task_control'",
        "'task050_proceed_requires_current_writer_and_retries_without_currentness_when_provisioned'",
        "'--', '--exact', '--nocapture', '--test-threads=1'",
        'LATTICE_STORE_DAEMON_INSTANCE_ID',
        'LATTICE_STORE_DAEMON_EPOCH',
        'LATTICE_STORE_AUTHORITY_REVISION',
        'LATTICE_STORE_OBSERVATION_DIGEST',
        'LATTICE_STORE_AUTHORITY_HEAD_DIGEST',
        '-AuthorityProfile $authorityProfile',
        'TASK050_POSTGRES_PROCEED_WRITER_MATRIX_OK current=1 stale=1 wrong_fence=1 substituted=1 exact_retry=1',
        'TASK050_PROCEED_WRITER_MATRIX_OUTPUT_DELETE_FAILED'
    )) {
        if (-not $runnerSource.Contains($required) -or -not $matrixSource.Contains($required)) {
            throw 'TASK050_PROCEED_WRITER_MATRIX_SHAPE_REJECTED'
        }
    }
}

Assert-Task050EmbeddedHarnessUsesStoreOnly
Assert-Task050CanonicalProfileSelectorIsPrivateAndClosed
Assert-Task050ProceedWriterMatrixShape
if ($SelfTestOnly) {
    $powershell = (Get-Command -Name powershell.exe -CommandType Application -ErrorAction Stop |
        Select-Object -First 1).Source
    $runnerSelfTestOutput = @(
        & $powershell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File $runnerPath -SelfTestOnly 2>&1
    )
    if (
        $LASTEXITCODE -ne 0 -or
        @($runnerSelfTestOutput | Where-Object {
            [string]$_ -ceq 'TASK050_TASK019_SOURCE_TRANSFORM_SELF_TEST=PASS'
        }).Count -ne 1 -or
        @($runnerSelfTestOutput | Where-Object {
            [string]$_ -ceq 'TASK050_CANONICAL_LATTICED_RUNNER_SHAPE_SELF_TEST=PASS'
        }).Count -ne 1 -or
        @($runnerSelfTestOutput | Where-Object {
            [string]$_ -ceq 'TASK050_CANONICAL_LATTICED_SESSION_SELF_TEST=PASS'
        }).Count -ne 1 -or
        @($runnerSelfTestOutput | Where-Object {
            [string]$_ -ceq 'TASK050_LATTICED_STARTUP_DIAGNOSTIC_SELF_TEST=PASS'
        }).Count -ne 1 -or
        @($runnerSelfTestOutput | Where-Object {
            [string]$_ -ceq 'TASK050_PROCEED_WRITER_MATRIX_SHAPE_SELF_TEST=PASS'
        }).Count -ne 1 -or
        @($runnerSelfTestOutput | Where-Object {
            [string]$_ -ceq 'TASK050_AUTONOMY_ATOMICITY_FAULT_MATRIX_SHAPE_SELF_TEST=PASS'
        }).Count -ne 1
    ) {
        throw 'TASK050_CANONICAL_LATTICED_SESSION_SELF_TEST_REJECTED'
    }
    Write-Output 'TASK050_EMBEDDED_HARNESS_SELF_TEST=PASS'
    return
}
Push-Location $repositoryRoot
try {
    & cargo.exe fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'TASK050_FORMAT_REJECTED' }

    & cargo.exe clippy -p lattice-contracts -p lattice-orchestrator `
        -p lattice-task-ledger -p lattice-ports -p lattice-postgres-store `
        --all-targets --no-deps --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'TASK050_STRICT_CLIPPY_REJECTED' }

    # The three command-line allows cover frozen pre-existing categories at
    # historical composition/MCP symbols. New TASK-050 composition helpers are
    # separately focused-tested and have no unhandled warnings in other categories.
    & cargo.exe clippy -p lattice-runtime --all-targets --no-deps --locked -- `
        -D warnings -A clippy::too_many_arguments -A clippy::too_many_lines `
        -A clippy::needless_borrow
    if ($LASTEXITCODE -ne 0) { throw 'TASK050_RUNTIME_CLIPPY_REJECTED' }

    foreach ($test in @(
        @('-p', 'lattice-contracts', '--test', 'task_ingress_contracts'),
        @('-p', 'lattice-orchestrator', '--test', 'autonomy_control'),
        @('-p', 'lattice-orchestrator', '--test', 'controlled_task'),
        @('-p', 'lattice-task-ledger'),
        @('-p', 'lattice-ports'),
        @('-p', 'lattice-postgres-store', '--lib'),
        @('-p', 'lattice-postgres-store', '--test', 'migration_contract'),
        @('-p', 'lattice-runtime', '--lib'),
        @('-p', 'lattice-runtime', '--test', 'task_control'),
        @('-p', 'lattice-runtime', '--test', 'mcp')
    )) {
        & cargo.exe test @test --locked
        if ($LASTEXITCODE -ne 0) { throw 'TASK050_FOCUSED_RUST_REJECTED' }
    }

    & cargo.exe build -p lattice-runtime --bin latticed --locked
    if ($LASTEXITCODE -ne 0) { throw 'TASK050_LATTICED_BUILD_REJECTED' }

    & (Join-Path $PSScriptRoot 'run-task050-autonomy-receipt-acceptance.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'TASK050_POSTGRES_ACCEPTANCE_REJECTED' }

    & npm.cmd run check
    if ($LASTEXITCODE -ne 0) { throw 'TASK050_REPOSITORY_GATE_REJECTED' }
}
finally {
    Pop-Location
}

Write-Output 'TASK050_AUTONOMY_RECEIPT_ACCEPTANCE=PASS'
