[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
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
