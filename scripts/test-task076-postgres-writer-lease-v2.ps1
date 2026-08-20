[CmdletBinding()]
param(
    [switch]$SelfTestOnly,
    [switch]$StaticOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Task076LiveInvocationCount = 0
$script:Task076GlobalV5ManifestSha256 = 'f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d'
$script:Task076MemoryV3ManifestSha256 = 'd4cc712d262ae1f7c96bd65526eab611c90e193363afd865af2126307b2903f0'
$script:Task076WriterV2ManifestSha256 = '5f54c182465c8e2dc8a6e6cc2ebd9a375f776adf500656586e59bfbc7dfd31a4'
$script:Task076Task075AcceptanceMarker = 'TASK075_SCHEMA_V5_MIGRATION_RECONCILIATION_ACCEPTANCE=PASS'
$script:Task076AuthorityEnvironmentNames = @(
    'LATTICE_MEMORY_CATALOG_SIGNATURE_URL',
    'LATTICE_STORE_CATALOG_SIGNATURE_URL',
    'LATTICE_STORE_PROFILE_LIVE',
    'LATTICE_STORE_PROFILE_MIGRATOR_URL',
    'LATTICE_STORE_PROFILE_RUNTIME_URL',
    'LATTICE_TASK019_EXPECTED_MANIFEST',
    'LATTICE_TASK019_EXPECTED_UUID',
    'LATTICE_TASK019_HOLDER_CONSUMER_SESSION_ID',
    'LATTICE_TASK019_HOLDER_DEADLINE_UTC',
    'LATTICE_TASK019_HOLDER_NONCE',
    'LATTICE_TASK019_HOLDER_NONCE_COMMITMENT',
    'LATTICE_TASK019_HOLDER_RECEIPT_PATH',
    'LATTICE_TASK019_HOLDER_SESSION_ID',
    'LATTICE_TASK019_HOST',
    'LATTICE_TASK019_LIVE',
    'LATTICE_TASK019_PASSWORD',
    'LATTICE_TASK019_PHASE',
    'LATTICE_TASK019_PORT',
    'LATTICE_TASK019_RUN_ID',
    'LATTICE_TASK038_POSTGRES_PASSWORD',
    'LATTICE_TASK050_LIVE',
    'LATTICE_TASK075_CURRENT_CATALOG_ONLY',
    'LATTICE_TASK076_CATALOG_MEASURE',
    'LATTICE_TASK076_WRITER_PHASE',
    'LATTICE_WRITER_LEASE_ADMIN_URL',
    'LATTICE_WRITER_LEASE_ADMISSION_OBSERVATION_SHA256',
    'LATTICE_WRITER_LEASE_AUTHORITY_HEAD_SHA256',
    'LATTICE_WRITER_LEASE_AUTHORITY_REVISION',
    'LATTICE_WRITER_LEASE_DAEMON_EPOCH',
    'LATTICE_WRITER_LEASE_DAEMON_INSTANCE_ID',
    'LATTICE_WRITER_LEASE_DATABASE_IDENTITY_SHA256',
    'LATTICE_WRITER_LEASE_DATABASE_NAME',
    'LATTICE_WRITER_LEASE_GLOBAL_MANIFEST_SHA256',
    'LATTICE_WRITER_LEASE_MEMORY_MANIFEST_SHA256',
    'LATTICE_WRITER_LEASE_MIGRATOR_URL',
    'LATTICE_WRITER_LEASE_RUNTIME_URL'
)

if ($SelfTestOnly -and $StaticOnly) {
    throw 'TASK076_ACCEPTANCE_MODE_CONFLICT'
}

function Assert-Task076AuthorityEnvironmentVacant {
    foreach ($name in $script:Task076AuthorityEnvironmentNames) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if (-not [string]::IsNullOrEmpty($value)) {
            throw ('TASK076_AMBIENT_AUTHORITY_ENV_REJECTED_' + $name)
        }
    }
}

function Get-Task076GatePlan {
    return @(
        [pscustomobject][ordered]@{
            Name = 'FORMAT'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @('fmt', '--all', '--', '--check')
            FailureCode = 'TASK076_FORMAT_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'STRICT_CLIPPY'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @(
                'clippy', '-p', 'lattice-postgres-writer-lease',
                '-p', 'lattice-postgres-store',
                '-p', 'lattice-postgres-codebase-memory',
                '--all-targets', '--no-deps', '--locked', '--', '-D', 'warnings'
            )
            FailureCode = 'TASK076_STRICT_CLIPPY_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'WRITER_LEASE'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @('test', '-p', 'lattice-postgres-writer-lease', '--all-targets', '--locked')
            FailureCode = 'TASK076_WRITER_LEASE_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'STORE_MIGRATION_CONTRACT'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @('test', '-p', 'lattice-postgres-store', '--test', 'migration_contract', '--locked')
            FailureCode = 'TASK076_STORE_MIGRATION_CONTRACT_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'MEMORY'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @('test', '-p', 'lattice-postgres-codebase-memory', '--all-targets', '--locked')
            FailureCode = 'TASK076_MEMORY_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'TASK019_SELF_TEST'
            Type = 'SCRIPT'
            Script = 'scripts\run-task019-postgres.ps1'
            Arguments = @('-SelfTestOnly')
            FailureCode = 'TASK076_TASK019_SELF_TEST_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'TASK075_STATIC_REVALIDATION'
            Type = 'SCRIPT'
            Script = 'scripts\test-task075-schema-v5-migration-reconciliation.ps1'
            Arguments = @('-StaticOnly')
            FailureCode = 'TASK076_TASK075_STATIC_REVALIDATION_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'WRITER_V2_LIVE'
            Type = 'LIVE_SCRIPT'
            Script = 'scripts\run-task019-postgres.ps1'
            Arguments = @('-RunTask076WriterLeaseGate')
            FailureCode = 'TASK076_WRITER_V2_LIVE_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'TASK075_FULL_REVALIDATION'
            Type = 'LIVE_SCRIPT'
            Script = 'scripts\test-task075-schema-v5-migration-reconciliation.ps1'
            Arguments = @()
            FailureCode = 'TASK076_TASK075_FULL_REVALIDATION_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'PROJECT_CHECK'
            Type = 'COMMAND'
            Command = 'npm.cmd'
            Arguments = @('run', 'check')
            FailureCode = 'TASK076_PROJECT_CHECK_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'DIFF_CHECK'
            Type = 'COMMAND'
            Command = 'git'
            Arguments = @('diff', '--check')
            FailureCode = 'TASK076_DIFF_CHECK_REJECTED'
        }
    )
}

function Invoke-Task076Gate {
    param(
        [Parameter(Mandatory = $true)]$Gate,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    Write-Output ('TASK076_GATE_ENTER_' + [string]$Gate.Name)
    $output = @()
    $exitCode = 0
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        if ([string]$Gate.Type -eq 'COMMAND') {
            $output = @(& ([string]$Gate.Command) @($Gate.Arguments) 2>&1)
        }
        else {
            $scriptPath = Join-Path $RepositoryRoot ([string]$Gate.Script)
            if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
                throw ('TASK076_GATE_SCRIPT_MISSING_' + [string]$Gate.Name)
            }
            if ([string]$Gate.Type -eq 'LIVE_SCRIPT') {
                $script:Task076LiveInvocationCount++
            }
            $output = @(& powershell.exe '-NoProfile' '-ExecutionPolicy' 'Bypass' '-File' $scriptPath @($Gate.Arguments) 2>&1)
        }
        $exitCode = [int]$LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if ($exitCode -ne 0) {
        throw ([string]$Gate.FailureCode)
    }
    Write-Output ('TASK076_GATE_PASS_' + [string]$Gate.Name)
    return $output
}

function Get-Task076SingleMarkerValue {
    param(
        [Parameter(Mandatory = $true)][object[]]$Output,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Pattern
    )
    $matches = @($Output | ForEach-Object { [string]$_ } | Where-Object { $_ -cmatch ('^' + [regex]::Escape($Name) + '=') })
    if ($matches.Count -ne 1) {
        throw ('TASK076_MARKER_SHAPE_REJECTED_' + $Name)
    }
    $value = $matches[0].Substring($Name.Length + 1)
    if ($value -cnotmatch $Pattern) {
        throw ('TASK076_MARKER_VALUE_REJECTED_' + $Name)
    }
    return $value
}

function Confirm-Task076HolderReceipt {
    param(
        [Parameter(Mandatory = $true)][object[]]$Output,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $path = Get-Task076SingleMarkerValue -Output $Output -Name 'HOLDER_RECEIPT_PATH' -Pattern '^.+\.jsonl$'
    $rawSha = Get-Task076SingleMarkerValue -Output $Output -Name 'HOLDER_RECEIPT_RAW_SHA256' -Pattern '^[a-f0-9]{64}$'
    $finalHmac = Get-Task076SingleMarkerValue -Output $Output -Name 'HOLDER_RECEIPT_FINAL_HMAC_SHA256' -Pattern '^[a-f0-9]{64}$'
    $eventCountText = Get-Task076SingleMarkerValue -Output $Output -Name 'HOLDER_RECEIPT_EVENT_COUNT' -Pattern '^[0-9]+$'
    $resolvedPath = [System.IO.Path]::GetFullPath($path)
    $receiptRoot = [System.IO.Path]::GetFullPath((Join-Path $RepositoryRoot 'target\task019-holder-receipts'))
    if (-not $resolvedPath.StartsWith($receiptRoot + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'TASK076_RECEIPT_PATH_REJECTED'
    }
    if (-not (Test-Path -LiteralPath $resolvedPath -PathType Leaf)) {
        throw 'TASK076_RECEIPT_MISSING'
    }
    $actualRawSha = (Get-FileHash -LiteralPath $resolvedPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualRawSha -cne $rawSha) {
        throw 'TASK076_RECEIPT_RAW_SHA_REJECTED'
    }
    $events = @(Get-Content -LiteralPath $resolvedPath -Encoding utf8 | ForEach-Object { $_ | ConvertFrom-Json })
    $expectedEventTypes = @(
        'HOLDER_OPEN',
        'MARKER_CREATED',
        'INITIAL_POSTMASTER_READY',
        'INITIAL_POSTMASTER_STOPPED',
        'RESTART_POSTMASTER_READY',
        'TASK076_WRITER_V2_VERIFIED',
        'HOLDER_STOP_REQUESTED',
        'HOLDER_STOPPED',
        'CLEANUP_REQUESTED',
        'CLEANUP_COMPLETED',
        'RECEIPT_CLOSED'
    )
    if (
        $events.Count -ne [int]$eventCountText -or
        $events.Count -ne $expectedEventTypes.Count -or
        (@($events | ForEach-Object { [string]$_.event_type }) -join '|') -cne
            ($expectedEventTypes -join '|')
    ) {
        throw 'TASK076_RECEIPT_EVENT_COUNT_REJECTED'
    }
    $previousHmac = ('0' * 64)
    for ($index = 0; $index -lt $events.Count; $index++) {
        $event = $events[$index]
        if ([long]$event.ordinal -ne ($index + 1)) {
            throw 'TASK076_RECEIPT_ORDINAL_REJECTED'
        }
        if ([string]$event.previous_hmac_sha256 -cne $previousHmac) {
            throw 'TASK076_RECEIPT_CHAIN_REJECTED'
        }
        if ([string]$event.event_hmac_sha256 -cnotmatch '^[a-f0-9]{64}$') {
            throw 'TASK076_RECEIPT_HMAC_SHAPE_REJECTED'
        }
        $previousHmac = [string]$event.event_hmac_sha256
    }
    if ($previousHmac -cne $finalHmac) {
        throw 'TASK076_RECEIPT_FINAL_HMAC_REJECTED'
    }
    $verified = $events[5]
    $cleanup = $events[-2]
    $final = $events[-1]
    if (
        [string]$verified.event_type -cne 'TASK076_WRITER_V2_VERIFIED' -or
        [string]$verified.payload.source_history_sha256 -cnotmatch '^[a-f0-9]{64}$' -or
        [string]$verified.payload.runtime_history_sha256 -cnotmatch '^[a-f0-9]{64}$' -or
        [string]$verified.payload.source_history_sha256 -ceq [string]$verified.payload.runtime_history_sha256 -or
        [long]$verified.payload.fencing_high_water -ne 2 -or
        [long]$verified.payload.command_high_water -ne 4 -or
        [long]$verified.payload.transition_high_water -ne 4 -or
        -not [bool]$verified.payload.physical_restart_verified -or
        [string]$verified.payload.upgrade_database_uuid -cnotmatch `
            '^[0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$' -or
        [string]$verified.payload.fresh_database_name -cnotmatch `
            '^lattice_task019_[0-9a-f]{8}_writer_fresh$' -or
        [string]$verified.payload.fresh_database_uuid -cnotmatch `
            '^[0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$' -or
        [string]$verified.payload.fresh_database_uuid -ceq `
            [string]$verified.payload.upgrade_database_uuid -or
        [string]$verified.payload.fresh_global_manifest_sha256 -cne `
            $script:Task076GlobalV5ManifestSha256 -or
        [string]$verified.payload.fresh_memory_manifest_sha256 -cne `
            $script:Task076MemoryV3ManifestSha256 -or
        [string]$verified.payload.fresh_writer_manifest_sha256 -cne `
            $script:Task076WriterV2ManifestSha256 -or
        [string]$verified.payload.fresh_ledger_shape -cne '1:INSTALLED' -or
        [string]$verified.payload.fresh_initial_profile_sha256 -cnotmatch '^[a-f0-9]{64}$' -or
        [string]$verified.payload.fresh_restart_profile_sha256 -cne `
            [string]$verified.payload.fresh_initial_profile_sha256 -or
        [string]$verified.payload.fresh_initial_install_outcome -cne 'INSTALLED' -or
        [string]$verified.payload.fresh_initial_reapply_outcome -cne 'ALREADY_CURRENT' -or
        [string]$verified.payload.fresh_restart_reapply_outcome -cne 'ALREADY_CURRENT' -or
        -not [bool]$verified.payload.fresh_physical_restart_verified -or
        [string]$cleanup.event_type -cne 'CLEANUP_COMPLETED' -or
        -not [bool]$cleanup.payload.cluster_root_absent -or
        -not [bool]$cleanup.payload.listener_absent -or
        [string]$final.event_type -cne 'RECEIPT_CLOSED' -or
        -not [bool]$final.payload.cleanup_complete
    ) {
        throw 'TASK076_RECEIPT_CLOSURE_REJECTED'
    }
}

function Invoke-Task076SelfTest {
    $plan = @(Get-Task076GatePlan)
    $expectedNames = @(
        'FORMAT', 'STRICT_CLIPPY', 'WRITER_LEASE', 'STORE_MIGRATION_CONTRACT',
        'MEMORY', 'TASK019_SELF_TEST', 'TASK075_STATIC_REVALIDATION',
        'WRITER_V2_LIVE', 'TASK075_FULL_REVALIDATION', 'PROJECT_CHECK', 'DIFF_CHECK'
    )
    if (($plan.Name -join '|') -cne ($expectedNames -join '|')) {
        throw 'TASK076_GATE_PLAN_REJECTED'
    }
    $live = @($plan | Where-Object { [string]$_.Type -eq 'LIVE_SCRIPT' })
    if (
        $live.Count -ne 2 -or
        ([string]$live[0].Arguments[0]) -cne '-RunTask076WriterLeaseGate' -or
        [string]$live[1].Name -cne 'TASK075_FULL_REVALIDATION'
    ) {
        throw 'TASK076_LIVE_PLAN_REJECTED'
    }
    $task075Source = [Text.UTF8Encoding]::new($false, $true).GetString(
        [IO.File]::ReadAllBytes((Join-Path $PSScriptRoot 'test-task075-schema-v5-migration-reconciliation.ps1'))
    )
    $task075MarkerLiteral = "Write-Output '$($script:Task076Task075AcceptanceMarker)'"
    if ([regex]::Matches($task075Source, [regex]::Escape($task075MarkerLiteral)).Count -ne 1) {
        throw 'TASK076_TASK075_ACCEPTANCE_MARKER_SELF_TEST_REJECTED'
    }
    foreach ($authorityName in @(
        'LATTICE_WRITER_LEASE_RUNTIME_URL',
        'LATTICE_TASK076_WRITER_PHASE',
        'LATTICE_TASK076_CATALOG_MEASURE'
    )) {
        $original = [Environment]::GetEnvironmentVariable($authorityName, 'Process')
        try {
            [Environment]::SetEnvironmentVariable($authorityName, 'sentinel', 'Process')
            try {
                Assert-Task076AuthorityEnvironmentVacant
                throw 'TASK076_AUTHORITY_SELF_TEST_FALSE_PASS'
            }
            catch {
                if ($_.Exception.Message -cne "TASK076_AMBIENT_AUTHORITY_ENV_REJECTED_$authorityName") {
                    throw
                }
            }
        }
        finally {
            [Environment]::SetEnvironmentVariable($authorityName, $original, 'Process')
        }
    }
    $runTask019Source = Get-Content -LiteralPath (
        Join-Path $repositoryRoot 'scripts\run-task019-postgres.ps1'
    ) -Raw -Encoding utf8
    foreach ($required in @(
        'task076_writer_fresh_setup',
        'task076_memory_fresh_setup',
        "-Phase 'fresh_install'",
        "-Phase 'fresh_restart'",
        'TASK076_WRITER_LEASE_V2_FRESH_CURRENT=PASS'
    )) {
        if (-not $runTask019Source.Contains($required)) {
            throw ('TASK076_FRESH_WIRING_SELF_TEST_REJECTED_' + $required)
        }
    }
    $phaseFunctionStart = $runTask019Source.IndexOf('function Invoke-Task076WriterLeaseGatePhase')
    $phaseFunctionEnd = $runTask019Source.IndexOf('function Get-PgIsReadyExitCode', $phaseFunctionStart)
    if ($phaseFunctionStart -lt 0 -or $phaseFunctionEnd -le $phaseFunctionStart) {
        throw 'TASK076_FRESH_PHASE_FUNCTION_SELF_TEST_REJECTED'
    }
    $phaseFunction = $runTask019Source.Substring(
        $phaseFunctionStart,
        $phaseFunctionEnd - $phaseFunctionStart
    )
    $initialOrder = @(
        "-Phase 'task076_final_verify'",
        "-Phase 'task076_writer_fresh_setup'",
        "-Phase 'task076_memory_fresh_setup'",
        "-Phase 'fresh_install'",
        "-Phase 'task076_writer_base_access'"
    ) | ForEach-Object { $phaseFunction.IndexOf($_) }
    $restartOrder = @(
        "-Phase 'task076_writer_restart'",
        "-Phase 'task076_writer_fresh_access'",
        "-Phase 'fresh_restart'"
    ) | ForEach-Object { $phaseFunction.IndexOf($_) }
    if (
        @($initialOrder | Where-Object { $_ -lt 0 }).Count -ne 0 -or
        @($restartOrder | Where-Object { $_ -lt 0 }).Count -ne 0 -or
        ($initialOrder -join ',') -cne (($initialOrder | Sort-Object) -join ',') -or
        ($restartOrder -join ',') -cne (($restartOrder | Sort-Object) -join ',') -or
        [regex]::Matches(
            $phaseFunction,
            [regex]::Escape("-Phase 'task076_writer_base_access'")
        ).Count -ne 2 -or
        [regex]::Matches(
            $phaseFunction,
            [regex]::Escape("-Phase 'task076_writer_fresh_access'")
        ).Count -ne 1
    ) {
        throw 'TASK076_FRESH_PHASE_ORDER_SELF_TEST_REJECTED'
    }
    $currentMeasurement = $phaseFunction.IndexOf("if (`$MeasurementProfile -ceq 'current')")
    $freshSetup = $phaseFunction.IndexOf("-Phase 'task076_writer_fresh_setup'")
    if (
        $currentMeasurement -lt 0 -or $freshSetup -le $currentMeasurement -or
        $phaseFunction.Substring($currentMeasurement, $freshSetup - $currentMeasurement) `
            -cnotmatch 'return \$output'
    ) {
        throw 'TASK076_FRESH_CATALOG_EARLY_RETURN_SELF_TEST_REJECTED'
    }
    $storeLiveSource = Get-Content -LiteralPath (
        Join-Path $repositoryRoot 'crates\lattice-postgres-store\tests\postgres_live.rs'
    ) -Raw -Encoding utf8
    $accessFunctionStart = $storeLiveSource.IndexOf('fn run_task076_writer_access_phase')
    $accessFunctionEnd = $storeLiveSource.IndexOf("`nfn run_initial_phase", $accessFunctionStart)
    if (
        $accessFunctionStart -lt 0 -or $accessFunctionEnd -le $accessFunctionStart -or
        $storeLiveSource.Contains('set_exact_task076_database_access')
    ) {
        throw 'TASK076_SINGLE_TARGET_ACCESS_FUNCTION_SELF_TEST_REJECTED'
    }
    $accessFunction = $storeLiveSource.Substring(
        $accessFunctionStart,
        $accessFunctionEnd - $accessFunctionStart
    )
    if (
        -not $accessFunction.Contains('matches!(database_tag, "base" | "writer_fresh")') -or
        [regex]::Matches(
            $accessFunction,
            [regex]::Escape('set_exact_database_access(&mut admin, &database_name)')
        ).Count -ne 1
    ) {
        throw 'TASK076_SINGLE_TARGET_ACCESS_SWITCH_SELF_TEST_REJECTED'
    }
    Write-Output 'TASK076_ACCEPTANCE_SELF_TEST=PASS'
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $repositoryRoot

if ($SelfTestOnly) {
    Invoke-Task076SelfTest
    exit 0
}

Assert-Task076AuthorityEnvironmentVacant
$plan = @(Get-Task076GatePlan)
foreach ($gate in $plan) {
    if ($StaticOnly -and [string]$gate.Type -eq 'LIVE_SCRIPT') {
        continue
    }
    $gateOutput = @(Invoke-Task076Gate -Gate $gate -RepositoryRoot $repositoryRoot)
    if ([string]$gate.Name -eq 'WRITER_V2_LIVE') {
        $text = $gateOutput -join "`n"
        if (
            $text -notmatch '(?m)^TASK076_WRITER_LEASE_V2_BRIDGE=PASS\s*$' -or
            $text -notmatch '(?m)^TASK076_WRITER_LEASE_V2_FRESH_CURRENT=PASS\s*$' -or
            $text -notmatch '(?m)^TASK019_POSTGRES_HARNESS=PASS\s*$' -or
            $text -match '(?m)(?:^|[^\S\r\n])SKIP:'
        ) {
            throw 'TASK076_WRITER_V2_LIVE_MARKER_REJECTED'
        }
        Confirm-Task076HolderReceipt -Output $gateOutput -RepositoryRoot $repositoryRoot
    }
    if ([string]$gate.Name -eq 'TASK075_FULL_REVALIDATION') {
        if (@($gateOutput | Where-Object {
            [string]$_ -ceq $script:Task076Task075AcceptanceMarker
        }).Count -ne 1) {
            throw 'TASK076_TASK075_REVALIDATION_MARKER_REJECTED'
        }
    }
}

if ($StaticOnly) {
    if ($script:Task076LiveInvocationCount -ne 0) {
        throw 'TASK076_STATIC_MODE_LIVE_INVOCATION_REJECTED'
    }
    Write-Output 'TASK076_LIVE_GATE_ENTER_COUNT=0'
    Write-Output 'TASK076_WRITER_LEASE_V2_STATIC_GATES=PASS'
}
else {
    if ($script:Task076LiveInvocationCount -ne 2) {
        throw 'TASK076_LIVE_INVOCATION_COUNT_REJECTED'
    }
    Write-Output 'TASK076_WRITER_LEASE_V2_BRIDGE_ACCEPTANCE=PASS'
}
