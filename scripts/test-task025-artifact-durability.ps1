[CmdletBinding()]
param(
    [switch]$SelfTestOnly,
    [switch]$StaticOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Task025LiveInvocationCount = 0
$script:Task025AuthorityEnvironmentNames = @(
    'LATTICE_TASK019_HOST',
    'LATTICE_TASK019_PORT',
    'LATTICE_TASK019_PASSWORD',
    'LATTICE_TASK019_RUN_ID',
    'LATTICE_TASK019_HOLDER_RECEIPT_PATH',
    'LATTICE_TASK019_HOLDER_NONCE',
    'LATTICE_TASK025_ARTIFACT_LIVE',
    'LATTICE_TASK025_ARTIFACT_PHASE',
    'LATTICE_ARTIFACT_MIGRATOR_URL',
    'LATTICE_ARTIFACT_RUNTIME_URL',
    'LATTICE_ARTIFACT_DATABASE_NAME',
    'LATTICE_ARTIFACT_DATABASE_IDENTITY_SHA256',
    'LATTICE_ARTIFACT_GLOBAL_MANIFEST_SHA256',
    'LATTICE_ARTIFACT_MEMORY_MANIFEST_SHA256',
    'LATTICE_WRITER_LEASE_MIGRATOR_URL',
    'LATTICE_WRITER_LEASE_RUNTIME_URL',
    'LATTICE_WRITER_LEASE_DATABASE_NAME'
)

if ($SelfTestOnly -and $StaticOnly) {
    throw 'TASK025_ACCEPTANCE_MODE_CONFLICT'
}

function Assert-Task025AuthorityEnvironmentVacant {
    foreach ($name in $script:Task025AuthorityEnvironmentNames) {
        if (-not [string]::IsNullOrEmpty(
            [Environment]::GetEnvironmentVariable($name, 'Process')
        )) {
            throw ('TASK025_AMBIENT_AUTHORITY_ENV_REJECTED_' + $name)
        }
    }
}

function Get-Task025GatePlan {
    return @(
        [pscustomobject][ordered]@{
            Name = 'FORMAT'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @('fmt', '--all', '--', '--check')
            FailureCode = 'TASK025_FORMAT_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'STRICT_CLIPPY'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @(
                'clippy', '-p', 'lattice-artifact-store',
                '-p', 'lattice-postgres-artifact-store',
                '-p', 'lattice-artifact-owned-root',
                '--all-targets', '--no-deps', '--locked', '--', '-D', 'warnings'
            )
            FailureCode = 'TASK025_STRICT_CLIPPY_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'DOMAIN_AND_ADAPTERS'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @(
                'test', '-p', 'lattice-artifact-store',
                '-p', 'lattice-postgres-artifact-store',
                '-p', 'lattice-artifact-owned-root',
                '--all-targets', '--locked'
            )
            FailureCode = 'TASK025_DOMAIN_AND_ADAPTERS_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'TASK019_SELF_TEST'
            Type = 'SCRIPT'
            Script = 'scripts\run-task019-postgres.ps1'
            Arguments = @('-SelfTestOnly')
            FailureCode = 'TASK025_TASK019_SELF_TEST_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'ARTIFACT_LIVE'
            Type = 'LIVE_SCRIPT'
            Script = 'scripts\run-task019-postgres.ps1'
            Arguments = @('-RunTask025ArtifactGate')
            FailureCode = 'TASK025_ARTIFACT_LIVE_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'PROJECT_CHECK'
            Type = 'COMMAND'
            Command = 'npm.cmd'
            Arguments = @('run', 'check')
            FailureCode = 'TASK025_PROJECT_CHECK_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'DIFF_CHECK'
            Type = 'COMMAND'
            Command = 'git'
            Arguments = @('diff', '--check')
            FailureCode = 'TASK025_DIFF_CHECK_REJECTED'
        }
    )
}

function Invoke-Task025Gate {
    param(
        [Parameter(Mandatory = $true)]$Gate,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    Write-Output ('TASK025_GATE_ENTER_' + [string]$Gate.Name)
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        if ([string]$Gate.Type -eq 'COMMAND') {
            $output = @(& ([string]$Gate.Command) @($Gate.Arguments) 2>&1)
        }
        else {
            $scriptPath = Join-Path $RepositoryRoot ([string]$Gate.Script)
            if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
                throw ('TASK025_GATE_SCRIPT_MISSING_' + [string]$Gate.Name)
            }
            if ([string]$Gate.Type -eq 'LIVE_SCRIPT') {
                $script:Task025LiveInvocationCount++
            }
            $output = @(
                & powershell.exe '-NoProfile' '-ExecutionPolicy' 'Bypass' '-File' `
                    $scriptPath @($Gate.Arguments) 2>&1
            )
        }
        $exitCode = [int]$LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if ($exitCode -ne 0) {
        throw ([string]$Gate.FailureCode)
    }
    Write-Output ('TASK025_GATE_PASS_' + [string]$Gate.Name)
    return $output
}

function Get-Task025SingleMarkerValue {
    param(
        [Parameter(Mandatory = $true)][object[]]$Output,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Pattern
    )

    $matches = @(
        $Output | ForEach-Object { [string]$_ } |
            Where-Object { $_ -cmatch ('^' + [regex]::Escape($Name) + '=') }
    )
    if ($matches.Count -ne 1) {
        throw ('TASK025_MARKER_SHAPE_REJECTED_' + $Name)
    }
    $value = $matches[0].Substring($Name.Length + 1)
    if ($value -cnotmatch $Pattern) {
        throw ('TASK025_MARKER_VALUE_REJECTED_' + $Name)
    }
    return $value
}

function Confirm-Task025HolderReceipt {
    param(
        [Parameter(Mandatory = $true)][object[]]$Output,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $path = Get-Task025SingleMarkerValue -Output $Output `
        -Name 'HOLDER_RECEIPT_PATH' -Pattern '^.+\.jsonl$'
    $rawSha = Get-Task025SingleMarkerValue -Output $Output `
        -Name 'HOLDER_RECEIPT_RAW_SHA256' -Pattern '^[a-f0-9]{64}$'
    $eventCount = [int](Get-Task025SingleMarkerValue -Output $Output `
        -Name 'HOLDER_RECEIPT_EVENT_COUNT' -Pattern '^[0-9]+$')
    $resolvedPath = [IO.Path]::GetFullPath($path)
    $receiptRoot = [IO.Path]::GetFullPath(
        (Join-Path $RepositoryRoot 'target\task019-holder-receipts')
    )
    if (-not $resolvedPath.StartsWith(
        $receiptRoot + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase
    )) {
        throw 'TASK025_RECEIPT_PATH_REJECTED'
    }
    if ((Get-FileHash -LiteralPath $resolvedPath -Algorithm SHA256).Hash.ToLowerInvariant() `
        -cne $rawSha) {
        throw 'TASK025_RECEIPT_RAW_SHA_REJECTED'
    }
    $events = @(
        Get-Content -LiteralPath $resolvedPath -Encoding utf8 |
            ForEach-Object { $_ | ConvertFrom-Json }
    )
    $expected = @(
        'HOLDER_OPEN',
        'MARKER_CREATED',
        'INITIAL_POSTMASTER_READY',
        'INITIAL_POSTMASTER_STOPPED',
        'RESTART_POSTMASTER_READY',
        'TASK076_WRITER_V2_VERIFIED',
        'TASK025_ARTIFACT_VERIFIED',
        'HOLDER_STOP_REQUESTED',
        'HOLDER_STOPPED',
        'CLEANUP_REQUESTED',
        'CLEANUP_COMPLETED',
        'RECEIPT_CLOSED'
    )
    if ($events.Count -ne $eventCount -or ($events.event_type -join '|') -cne ($expected -join '|')) {
        throw 'TASK025_RECEIPT_EVENT_SEQUENCE_REJECTED'
    }
    return [pscustomobject]@{ Path = $resolvedPath; RawSha256 = $rawSha; EventCount = $eventCount }
}

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Assert-Task025AuthorityEnvironmentVacant
$plan = @(Get-Task025GatePlan)
if ($plan.Count -ne 7 -or @($plan | Where-Object Type -eq 'LIVE_SCRIPT').Count -ne 1) {
    throw 'TASK025_GATE_PLAN_REJECTED'
}
if ($SelfTestOnly) {
    Write-Output 'TASK025_ACCEPTANCE_SELF_TEST=PASS'
    return
}

$liveOutput = @()
foreach ($gate in $plan) {
    if ($StaticOnly -and [string]$gate.Type -eq 'LIVE_SCRIPT') {
        continue
    }
    $output = @(Invoke-Task025Gate -Gate $gate -RepositoryRoot $repositoryRoot)
    if ([string]$gate.Type -eq 'LIVE_SCRIPT') {
        $liveOutput = $output
    }
}
if ($StaticOnly) {
    if ($script:Task025LiveInvocationCount -ne 0) {
        throw 'TASK025_STATIC_MODE_LIVE_INVOKED'
    }
    Write-Output 'TASK025_ARTIFACT_DURABILITY_STATIC=PASS'
    return
}
if ($script:Task025LiveInvocationCount -ne 1) {
    throw 'TASK025_LIVE_INVOCATION_COUNT_REJECTED'
}
$text = @($liveOutput | ForEach-Object { [string]$_ }) -join "`n"
foreach ($token in @(
    'TASK025_ARTIFACT_POSTGRES_PROFILE=PASS',
    'TASK019_POSTGRES_HARNESS=PASS'
)) {
    if ([regex]::Matches($text, '(?m)^' + [regex]::Escape($token) + '$').Count -ne 1) {
        throw ('TASK025_LIVE_TOKEN_REJECTED_' + $token)
    }
}
$receipt = Confirm-Task025HolderReceipt -Output $liveOutput -RepositoryRoot $repositoryRoot
Write-Output ('HOLDER_RECEIPT_PATH=' + [string]$receipt.Path)
Write-Output ('HOLDER_RECEIPT_RAW_SHA256=' + [string]$receipt.RawSha256)
Write-Output ('HOLDER_RECEIPT_EVENT_COUNT=' + [string]$receipt.EventCount)
Write-Output 'TASK025_ARTIFACT_DURABILITY_ACCEPTANCE=PASS'
