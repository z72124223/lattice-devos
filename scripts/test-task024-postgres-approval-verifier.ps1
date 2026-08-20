[CmdletBinding()]
param(
    [switch]$SelfTestOnly,
    [switch]$StaticOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Task024LiveInvocationCount = 0
$script:Task024AuthorityEnvironmentNames = @(
    'LATTICE_TASK019_HOST',
    'LATTICE_TASK019_PORT',
    'LATTICE_TASK019_PASSWORD',
    'LATTICE_TASK019_RUN_ID',
    'LATTICE_TASK019_HOLDER_RECEIPT_PATH',
    'LATTICE_TASK019_HOLDER_NONCE',
    'LATTICE_TASK024_APPROVAL_LIVE',
    'LATTICE_TASK024_APPROVAL_PHASE',
    'LATTICE_APPROVAL_MIGRATOR_URL',
    'LATTICE_APPROVAL_RUNTIME_URL',
    'LATTICE_APPROVAL_DATABASE_NAME',
    'LATTICE_APPROVAL_DATABASE_IDENTITY_SHA256',
    'LATTICE_APPROVAL_GLOBAL_MANIFEST_SHA256',
    'LATTICE_APPROVAL_MEMORY_MANIFEST_SHA256',
    'LATTICE_WRITER_LEASE_MIGRATOR_URL',
    'LATTICE_WRITER_LEASE_RUNTIME_URL',
    'LATTICE_WRITER_LEASE_DATABASE_NAME'
)

if ($SelfTestOnly -and $StaticOnly) {
    throw 'TASK024_ACCEPTANCE_MODE_CONFLICT'
}

function Assert-Task024AuthorityEnvironmentVacant {
    foreach ($name in $script:Task024AuthorityEnvironmentNames) {
        if (-not [string]::IsNullOrEmpty(
            [Environment]::GetEnvironmentVariable($name, 'Process')
        )) {
            throw ('TASK024_AMBIENT_AUTHORITY_ENV_REJECTED_' + $name)
        }
    }
}

function Get-Task024GatePlan {
    return @(
        [pscustomobject][ordered]@{
            Name = 'FORMAT'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @('fmt', '--all', '--', '--check')
            FailureCode = 'TASK024_FORMAT_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'STRICT_CLIPPY'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @(
                'clippy', '-p', 'lattice-approval-verifier',
                '-p', 'lattice-postgres-approval-verifier',
                '--all-targets', '--no-deps', '--locked', '--', '-D', 'warnings'
            )
            FailureCode = 'TASK024_STRICT_CLIPPY_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'DOMAIN_AND_ADAPTER'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @(
                'test', '-p', 'lattice-approval-verifier',
                '-p', 'lattice-postgres-approval-verifier',
                '--all-targets', '--locked'
            )
            FailureCode = 'TASK024_DOMAIN_AND_ADAPTER_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'TASK019_SELF_TEST'
            Type = 'SCRIPT'
            Script = 'scripts\run-task019-postgres.ps1'
            Arguments = @('-SelfTestOnly')
            FailureCode = 'TASK024_TASK019_SELF_TEST_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'APPROVAL_LIVE'
            Type = 'LIVE_SCRIPT'
            Script = 'scripts\run-task019-postgres.ps1'
            Arguments = @('-RunTask024ApprovalGate')
            FailureCode = 'TASK024_APPROVAL_LIVE_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'PROJECT_CHECK'
            Type = 'COMMAND'
            Command = 'npm.cmd'
            Arguments = @('run', 'check')
            FailureCode = 'TASK024_PROJECT_CHECK_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'DIFF_CHECK'
            Type = 'COMMAND'
            Command = 'git'
            Arguments = @('diff', '--check')
            FailureCode = 'TASK024_DIFF_CHECK_REJECTED'
        }
    )
}

function Invoke-Task024Gate {
    param(
        [Parameter(Mandatory = $true)]$Gate,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    Write-Output ('TASK024_GATE_ENTER_' + [string]$Gate.Name)
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        if ([string]$Gate.Type -eq 'COMMAND') {
            $output = @(& ([string]$Gate.Command) @($Gate.Arguments) 2>&1)
        }
        else {
            $scriptPath = Join-Path $RepositoryRoot ([string]$Gate.Script)
            if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
                throw ('TASK024_GATE_SCRIPT_MISSING_' + [string]$Gate.Name)
            }
            if ([string]$Gate.Type -eq 'LIVE_SCRIPT') {
                $script:Task024LiveInvocationCount++
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
    Write-Output ('TASK024_GATE_PASS_' + [string]$Gate.Name)
    return $output
}

function Get-Task024SingleMarkerValue {
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
        throw ('TASK024_MARKER_SHAPE_REJECTED_' + $Name)
    }
    $value = $matches[0].Substring($Name.Length + 1)
    if ($value -cnotmatch $Pattern) {
        throw ('TASK024_MARKER_VALUE_REJECTED_' + $Name)
    }
    return $value
}

function Confirm-Task024HolderReceipt {
    param(
        [Parameter(Mandatory = $true)][object[]]$Output,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $path = Get-Task024SingleMarkerValue -Output $Output `
        -Name 'HOLDER_RECEIPT_PATH' -Pattern '^.+\.jsonl$'
    $rawSha = Get-Task024SingleMarkerValue -Output $Output `
        -Name 'HOLDER_RECEIPT_RAW_SHA256' -Pattern '^[a-f0-9]{64}$'
    $finalHmac = Get-Task024SingleMarkerValue -Output $Output `
        -Name 'HOLDER_RECEIPT_FINAL_HMAC_SHA256' -Pattern '^[a-f0-9]{64}$'
    $eventCount = [int](Get-Task024SingleMarkerValue -Output $Output `
        -Name 'HOLDER_RECEIPT_EVENT_COUNT' -Pattern '^[0-9]+$')
    $resolvedPath = [IO.Path]::GetFullPath($path)
    $receiptRoot = [IO.Path]::GetFullPath(
        (Join-Path $RepositoryRoot 'target\task019-holder-receipts')
    )
    if (-not $resolvedPath.StartsWith(
        $receiptRoot + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase
    )) {
        throw 'TASK024_RECEIPT_PATH_REJECTED'
    }
    if ((Get-FileHash -LiteralPath $resolvedPath -Algorithm SHA256).Hash.ToLowerInvariant() `
        -cne $rawSha) {
        throw 'TASK024_RECEIPT_RAW_SHA_REJECTED'
    }
    $events = @(
        Get-Content -LiteralPath $resolvedPath -Encoding utf8 |
            ForEach-Object { $_ | ConvertFrom-Json }
    )
    $expectedEventTypes = @(
        'HOLDER_OPEN',
        'MARKER_CREATED',
        'INITIAL_POSTMASTER_READY',
        'INITIAL_POSTMASTER_STOPPED',
        'RESTART_POSTMASTER_READY',
        'TASK076_WRITER_V2_VERIFIED',
        'TASK024_APPROVAL_VERIFIED',
        'HOLDER_STOP_REQUESTED',
        'HOLDER_STOPPED',
        'CLEANUP_REQUESTED',
        'CLEANUP_COMPLETED',
        'RECEIPT_CLOSED'
    )
    if (
        $events.Count -ne $eventCount -or
        ($events.event_type -join '|') -cne ($expectedEventTypes -join '|')
    ) {
        throw 'TASK024_RECEIPT_EVENT_SEQUENCE_REJECTED'
    }
    $previousHmac = '0' * 64
    for ($index = 0; $index -lt $events.Count; $index++) {
        $event = $events[$index]
        if (
            [long]$event.ordinal -ne ($index + 1) -or
            [string]$event.previous_hmac_sha256 -cne $previousHmac -or
            [string]$event.event_hmac_sha256 -cnotmatch '^[a-f0-9]{64}$'
        ) {
            throw 'TASK024_RECEIPT_CHAIN_REJECTED'
        }
        $previousHmac = [string]$event.event_hmac_sha256
    }
    $verified = $events[6]
    if (
        $previousHmac -cne $finalHmac -or
        [string]$verified.payload.initial_approval_install -cne 'INSTALLED' -or
        [string]$verified.payload.initial_approval_reapply -cne 'ALREADY_CURRENT' -or
        [string]$verified.payload.restart_approval_reapply -cne 'ALREADY_CURRENT' -or
        -not [bool]$verified.payload.physical_restart_verified -or
        [string]$verified.payload.initial_restart_postmaster_started_at -ceq
            [string]$verified.payload.approval_restart_postmaster_started_at -or
        -not [bool]$events[-2].payload.cluster_root_absent -or
        -not [bool]$events[-2].payload.listener_absent -or
        -not [bool]$events[-1].payload.cleanup_complete
    ) {
        throw 'TASK024_RECEIPT_CLOSURE_REJECTED'
    }
}

function Invoke-Task024SelfTest {
    $plan = @(Get-Task024GatePlan)
    if (($plan.Name -join '|') -cne (
        'FORMAT|STRICT_CLIPPY|DOMAIN_AND_ADAPTER|TASK019_SELF_TEST|' +
        'APPROVAL_LIVE|PROJECT_CHECK|DIFF_CHECK'
    )) {
        throw 'TASK024_GATE_PLAN_REJECTED'
    }
    $live = @($plan | Where-Object { [string]$_.Type -eq 'LIVE_SCRIPT' })
    if (
        $live.Count -ne 1 -or
        ([string]$live[0].Arguments[0]) -cne '-RunTask024ApprovalGate'
    ) {
        throw 'TASK024_LIVE_PLAN_REJECTED'
    }
    $harnessSource = Get-Content -LiteralPath (
        Join-Path $repositoryRoot 'scripts\run-task019-postgres.ps1'
    ) -Raw -Encoding utf8
    foreach ($required in @(
        '[switch]$RunTask024ApprovalGate',
        'TASK024_APPROVAL_VERIFIED',
        "Write-Output 'TASK024_APPROVAL_POSTGRES_PROFILE=PASS'"
    )) {
        if (-not $harnessSource.Contains($required)) {
            throw ('TASK024_HARNESS_WIRING_REJECTED_' + $required)
        }
    }
    foreach ($authorityName in @(
        'LATTICE_APPROVAL_RUNTIME_URL',
        'LATTICE_TASK024_APPROVAL_PHASE'
    )) {
        $original = [Environment]::GetEnvironmentVariable($authorityName, 'Process')
        try {
            [Environment]::SetEnvironmentVariable($authorityName, 'sentinel', 'Process')
            try {
                Assert-Task024AuthorityEnvironmentVacant
                throw 'TASK024_AUTHORITY_SELF_TEST_FALSE_PASS'
            }
            catch {
                if ($_.Exception.Message -cne "TASK024_AMBIENT_AUTHORITY_ENV_REJECTED_$authorityName") {
                    throw
                }
            }
        }
        finally {
            [Environment]::SetEnvironmentVariable($authorityName, $original, 'Process')
        }
    }
    Write-Output 'TASK024_ACCEPTANCE_SELF_TEST=PASS'
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $repositoryRoot

if ($SelfTestOnly) {
    Invoke-Task024SelfTest
    exit 0
}

Assert-Task024AuthorityEnvironmentVacant
foreach ($gate in @(Get-Task024GatePlan)) {
    if ($StaticOnly -and [string]$gate.Type -eq 'LIVE_SCRIPT') {
        continue
    }
    $gateOutput = @(Invoke-Task024Gate -Gate $gate -RepositoryRoot $repositoryRoot)
    if ([string]$gate.Name -eq 'APPROVAL_LIVE') {
        $text = $gateOutput -join "`n"
        if (
            $text -notmatch '(?m)^TASK019_POSTGRES_HARNESS=PASS\s*$' -or
            $text -notmatch '(?m)^TASK024_APPROVAL_POSTGRES_PROFILE=PASS\s*$' -or
            $text -match '(?m)(?:^|[^\S\r\n])SKIP:'
        ) {
            throw 'TASK024_APPROVAL_LIVE_MARKER_REJECTED'
        }
        Confirm-Task024HolderReceipt -Output $gateOutput -RepositoryRoot $repositoryRoot
    }
}

if ($StaticOnly) {
    if ($script:Task024LiveInvocationCount -ne 0) {
        throw 'TASK024_STATIC_MODE_LIVE_INVOCATION_REJECTED'
    }
    Write-Output 'TASK024_LIVE_GATE_ENTER_COUNT=0'
    Write-Output 'TASK024_APPROVAL_STATIC_GATES=PASS'
}
else {
    if ($script:Task024LiveInvocationCount -ne 1) {
        throw 'TASK024_LIVE_INVOCATION_COUNT_REJECTED'
    }
    Write-Output 'TASK024_APPROVAL_DURABILITY_ACCEPTANCE=PASS'
}
