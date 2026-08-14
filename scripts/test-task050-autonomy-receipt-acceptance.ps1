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

Assert-Task050EmbeddedHarnessUsesStoreOnly
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
        }).Count -ne 1
    ) {
        throw 'TASK050_TASK019_SOURCE_TRANSFORM_SELF_TEST_REJECTED'
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

    # The three allows are existing runtime findings in unchanged composition/MCP
    # code; TASK-050's changed task_control slice remains warning-denied.
    & cargo.exe clippy -p lattice-runtime --all-targets --no-deps --locked -- `
        -D warnings -A clippy::too_many_arguments -A clippy::too_many_lines `
        -A clippy::needless_borrow
    if ($LASTEXITCODE -ne 0) { throw 'TASK050_RUNTIME_CLIPPY_REJECTED' }

    foreach ($test in @(
        @('-p', 'lattice-contracts', '--test', 'task_ingress_contracts'),
        @('-p', 'lattice-orchestrator', '--test', 'autonomy_control'),
        @('-p', 'lattice-orchestrator', '--test', 'controlled_task'),
        @('-p', 'lattice-task-ledger'),
        @('-p', 'lattice-postgres-store', '--lib'),
        @('-p', 'lattice-postgres-store', '--test', 'migration_contract'),
        @('-p', 'lattice-runtime', '--lib'),
        @('-p', 'lattice-runtime', '--test', 'task_control'),
        @('-p', 'lattice-runtime', '--test', 'mcp')
    )) {
        & cargo.exe test @test --locked
        if ($LASTEXITCODE -ne 0) { throw 'TASK050_FOCUSED_RUST_REJECTED' }
    }

    & (Join-Path $PSScriptRoot 'run-task050-autonomy-receipt-acceptance.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'TASK050_POSTGRES_ACCEPTANCE_REJECTED' }

    & npm.cmd run check
    if ($LASTEXITCODE -ne 0) { throw 'TASK050_REPOSITORY_GATE_REJECTED' }
}
finally {
    Pop-Location
}

Write-Output 'TASK050_AUTONOMY_RECEIPT_ACCEPTANCE=PASS'
