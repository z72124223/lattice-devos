[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$entrypoint = Join-Path $PSScriptRoot 'start-lattice-full-chain.ps1'
$cargo = @(Get-Command 'cargo.exe' -CommandType Application -ErrorAction Stop)[0]

function Invoke-CargoChecked {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)

    & $cargo.Source @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw 'LATTICE_OPERATOR_FOCUSED_TEST_FAILED'
    }
}

Push-Location $repositoryRoot
try {
    Invoke-CargoChecked -Arguments @(
        'test', '-p', 'lattice-runtime', '--test', 'composition',
        'real_latticed_binary_serves_only_the_two_bounded_tools', '--locked', '--', '--exact'
    )
    Invoke-CargoChecked -Arguments @(
        'test', '-p', 'lattice-runtime', '--test', 'composition',
        'full_chain_binary_is_reachable_and_fails_closed_without_a_sealed_hermes_runner',
        '--locked', '--', '--exact'
    )
    Invoke-CargoChecked -Arguments @(
        'test', '-p', 'lattice-runtime', '--lib',
        'full_chain_openclaw_pump_only_terminates_for_process_level_failures', '--locked'
    )
    Invoke-CargoChecked -Arguments @(
        'build', '-p', 'lattice-runtime', '--bin', 'latticed', '--bin', 'lattice-full-chain', '--locked'
    )

    $metadataText = (& $cargo.Source 'metadata' '--no-deps' '--format-version' '1' '--locked') -join "`n"
    if ($LASTEXITCODE -ne 0) {
        throw 'LATTICE_OPERATOR_METADATA_FAILED'
    }
    $metadata = $metadataText | ConvertFrom-Json
    $latticed = Join-Path ([string]$metadata.target_directory) 'debug\latticed.exe'

    $environment = [ordered]@{
        LATTICE_DELIVERY_CODEX_MODE = 'SCRIPTED_ACCEPTANCE'
        LATTICE_DELIVERY_LAUNCHER = 'C:\tools\codex.exe'
        LATTICE_DELIVERY_LAUNCHER_VERSION = 'codex-cli 0.144.6'
        LATTICE_DELIVERY_LAUNCHER_SHA256 = ('a' * 64)
        LATTICE_DELIVERY_SCHEMA_DIR = 'C:\delivery\schema'
        LATTICE_DELIVERY_CODEX_HOME = 'C:\delivery\codex-home'
        LATTICE_DELIVERY_ROOT = 'C:\delivery\root'
        LATTICE_DELIVERY_GIT_EXE = 'C:\tools\git.exe'
        LATTICE_TASK019_HOST = '127.0.0.1'
        LATTICE_TASK019_PORT = '55432'
        LATTICE_TASK019_RUN_ID = '0123456789abcdef0123456789abcdef'
        LATTICE_TASK019_PASSWORD = 'operator-smoke-test-password'
    }
    $original = @{}
    foreach ($entry in $environment.GetEnumerator()) {
        $original[$entry.Key] = [Environment]::GetEnvironmentVariable($entry.Key, 'Process')
        [Environment]::SetEnvironmentVariable($entry.Key, [string]$entry.Value, 'Process')
    }
    try {
        $smokeText = (& $entrypoint -Mode Smoke -ExecutablePath $latticed -SkipBuild -McpOnly) -join "`n"
    }
    finally {
        foreach ($entry in $environment.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable($entry.Key, $original[$entry.Key], 'Process')
        }
    }

    $smoke = $smokeText | ConvertFrom-Json
    if (
        [string]$smoke.status -ne 'PASS' -or
        -not [bool]$smoke.mcp_initialize -or
        (@($smoke.mcp_tools) -join ',') -ne 'lattice_delivery_run,lattice_delivery_status' -or
        [string]$smoke.openclaw_pump -ne 'NOT_CHECKED'
    ) {
        throw 'LATTICE_OPERATOR_SMOKE_EVIDENCE_REJECTED'
    }

    Write-Output 'LATTICE_OPERATOR_ENTRYPOINT_TEST=PASS'
}
finally {
    Pop-Location
}
