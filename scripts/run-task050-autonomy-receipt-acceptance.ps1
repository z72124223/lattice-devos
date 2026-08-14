[CmdletBinding()]
param(
    [switch]$SelfTestOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$task019Path = Join-Path $PSScriptRoot 'run-task019-postgres.ps1'
if (-not (Test-Path -LiteralPath $task019Path -PathType Leaf)) {
    throw 'TASK050_TASK019_HARNESS_MISSING'
}

$script:Task050CanonicalPhases = @('initial', 'restart')
$script:Task050AutonomyProfiles = @('ASK_USER', 'PROCEED')
$script:Task050ProfileTaskSpecDigests = [ordered]@{
    ASK_USER = '0915bc62fe4613bebda5a82e65863a325b7102124a61aa0efc9310a33a18be59'
    PROCEED = '0cdfb9ee77f8f3b819ddbd74bf2d58537da11ec065bb1526889bb08adf77e86d'
}
$script:Task050ExpectedTools = @(
    'lattice_delivery_run',
    'lattice_delivery_status',
    'lattice_task_status',
    'lattice_task_submit'
)
$script:Task050PublicStatusFields = @(
    'ledger_head_digest',
    'result_digest',
    'schema_version',
    'status',
    'task_ref',
    'task_state'
)
# The Rust live harness must emit exactly one JSON marker for each profile in
# each phase. The closed schema below binds the public six-field expectation,
# internal projection digest, ingress identity, and Store authority. Missing
# markers fail before any latticed process is resolved or started.
$script:Task050ProfileMarkerPrefix = 'TASK050_LATTICED_PROFILE_INPUT='
$script:Task050CanonicalExecutionImplemented = $true
$script:Task050ObservedProcessIds = [Collections.Generic.HashSet[int]]::new()
$script:Task050InitialProjectionDigests = @{}
$script:Task050InitialProfilePayloads = @{}

function Get-Task050StringSha256 {
    param([Parameter(Mandatory = $true)][string]$Value)

    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Value)
        return (($sha256.ComputeHash($bytes) | ForEach-Object { $_.ToString('x2') }) -join '')
    }
    finally {
        $sha256.Dispose()
    }
}

function Get-Task050HmacSha256 {
    param(
        [Parameter(Mandatory = $true)][string]$Key,
        [Parameter(Mandatory = $true)][string]$Value
    )

    $hmac = [Security.Cryptography.HMACSHA256]::new(
        [Text.UTF8Encoding]::new($false).GetBytes($Key)
    )
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Value)
        return (($hmac.ComputeHash($bytes) | ForEach-Object { $_.ToString('x2') }) -join '')
    }
    finally {
        $hmac.Dispose()
    }
}

function Resolve-Task050LatticedExecutable {
    $candidate = [IO.Path]::GetFullPath((Join-Path $repositoryRoot 'target\debug\latticed.exe'))
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw 'TASK050_LATTICED_EXECUTABLE_REJECTED'
    }
    $resolved = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $candidate -ErrorAction Stop).ProviderPath)
    if (
        [IO.Path]::GetFileName($resolved) -cne 'latticed.exe' -or
        -not (Test-Path -LiteralPath $resolved -PathType Leaf) -or
        -not $resolved.StartsWith(
            [IO.Path]::GetFullPath((Join-Path $repositoryRoot 'target')) + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase
        )
    ) {
        throw 'TASK050_LATTICED_EXECUTABLE_REJECTED'
    }
    return $resolved
}

function New-Task050LatticedProcessStartInfo {
    param(
        [Parameter(Mandatory = $true)][string]$LatticedExecutable
    )

    $fullyQualified = try {
        [IO.Path]::GetFullPath($LatticedExecutable) -ceq $LatticedExecutable
    } catch {
        $false
    }
    if (
        -not $fullyQualified -or
        [IO.Path]::GetFileName($LatticedExecutable) -cne 'latticed.exe'
    ) {
        throw 'TASK050_LATTICED_EXECUTABLE_REJECTED'
    }
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $LatticedExecutable
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $startInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    $startInfo.WorkingDirectory = $repositoryRoot
    return $startInfo
}

function Get-Task050PhaseProfileInputs {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('initial', 'restart')][string]$Phase,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$TestOutput
    )

    $lines = @($TestOutput | ForEach-Object { [string]$_ } | Where-Object {
        $_.StartsWith($script:Task050ProfileMarkerPrefix, [StringComparison]::Ordinal)
    })
    if ($lines.Count -ne 2) {
        throw 'TASK050_LATTICED_PROFILE_INPUT_REQUIRED'
    }
    $inputs = [Collections.Generic.List[object]]::new()
    foreach ($line in $lines) {
        try {
            $input = $line.Substring($script:Task050ProfileMarkerPrefix.Length) |
                ConvertFrom-Json -ErrorAction Stop
        }
        catch {
            throw 'TASK050_LATTICED_PROFILE_INPUT_REJECTED'
        }
        $expectedKeys = @(
            'authority_head_digest', 'authority_revision', 'autonomy_projection_sha256',
            'daemon_epoch', 'daemon_instance_id', 'database_run_id', 'expected_status',
            'ingress_profile_sha256', 'observation_digest', 'phase', 'profile', 'schema',
            'task_ref', 'task_spec_digest'
        ) | Sort-Object -CaseSensitive
        $actualKeys = @($input.PSObject.Properties.Name | Sort-Object -CaseSensitive)
        if (
            ($actualKeys -join "`n") -cne ($expectedKeys -join "`n") -or
            [string]$input.schema -cne 'lattice.task050.latticed-profile-input.v2' -or
            [string]$input.phase -cne $Phase -or
            [string]$input.profile -cnotmatch '\A(?:ASK_USER|PROCEED)\z' -or
            [string]$input.database_run_id -cnotmatch '\A[0-9a-f]{32}\z' -or
            [string]$input.task_ref -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$input.task_spec_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$input.autonomy_projection_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$input.ingress_profile_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$input.daemon_instance_id -cnotmatch '\A[a-z0-9][a-z0-9-]{0,63}\z' -or
            [long]$input.daemon_epoch -le 0 -or
            [long]$input.authority_revision -le 0 -or
            [string]$input.observation_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$input.authority_head_digest -cnotmatch '\A[0-9a-f]{64}\z'
        ) {
            throw 'TASK050_LATTICED_PROFILE_INPUT_REJECTED'
        }
        if (
            [string]$input.task_spec_digest -cne
                [string]$script:Task050ProfileTaskSpecDigests[[string]$input.profile]
        ) {
            throw 'TASK050_LATTICED_PROFILE_INPUT_REJECTED'
        }
        Assert-Task050SixFieldStatus -Status $input.expected_status
        if ([string]$input.expected_status.task_ref -cne [string]$input.task_ref) {
            throw 'TASK050_LATTICED_PROFILE_INPUT_REJECTED'
        }
        $inputs.Add($input)
    }
    $profiles = @($inputs | ForEach-Object { [string]$_.profile } | Sort-Object -CaseSensitive)
    if (($profiles -join ',') -cne 'ASK_USER,PROCEED') {
        throw 'TASK050_LATTICED_PROFILE_INPUT_REJECTED'
    }
    $databaseRunIds = @($inputs | ForEach-Object { [string]$_.database_run_id } |
        Sort-Object -CaseSensitive -Unique)
    if ($databaseRunIds.Count -ne 1) {
        throw 'TASK050_LATTICED_PROFILE_INPUT_REJECTED'
    }
    foreach ($input in $inputs) {
        $profile = [string]$input.profile
        $digest = [string]$input.autonomy_projection_sha256
        $durablePayload = [ordered]@{
            authority_head_digest = [string]$input.authority_head_digest
            authority_revision = [long]$input.authority_revision
            autonomy_projection_sha256 = $digest
            daemon_epoch = [long]$input.daemon_epoch
            daemon_instance_id = [string]$input.daemon_instance_id
            database_run_id = [string]$input.database_run_id
            expected_status = $input.expected_status
            ingress_profile_sha256 = [string]$input.ingress_profile_sha256
            observation_digest = [string]$input.observation_digest
            profile = $profile
            schema = [string]$input.schema
            task_ref = [string]$input.task_ref
            task_spec_digest = [string]$input.task_spec_digest
        } | ConvertTo-Json -Compress -Depth 6
        if ($Phase -ceq 'initial') {
            $script:Task050InitialProjectionDigests[$profile] = $digest
            $script:Task050InitialProfilePayloads[$profile] = $durablePayload
        }
        elseif (
            -not $script:Task050InitialProjectionDigests.ContainsKey($profile) -or
            [string]$script:Task050InitialProjectionDigests[$profile] -cne $digest -or
            -not $script:Task050InitialProfilePayloads.ContainsKey($profile) -or
            [string]$script:Task050InitialProfilePayloads[$profile] -cne $durablePayload
        ) {
            throw 'TASK050_LATTICED_PROFILE_REPLAY_REJECTED'
        }
    }
    return @($script:Task050AutonomyProfiles | ForEach-Object {
        $profile = $_
        @($inputs | Where-Object { [string]$_.profile -ceq $profile })[0]
    })
}

function New-Task050McpInput {
    param([Parameter(Mandatory = $true)][string]$TaskRef)

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'TASK050_MCP_INPUT_REJECTED'
    }
    $meta = [ordered]@{
        'io.modelcontextprotocol/protocolVersion' = '2026-07-28'
        'io.modelcontextprotocol/clientCapabilities' = [ordered]@{}
    }
    $frames = @(
        [ordered]@{ jsonrpc = '2.0'; id = 1; method = 'server/discover'; params = [ordered]@{ _meta = $meta } },
        [ordered]@{ jsonrpc = '2.0'; id = 2; method = 'tools/list'; params = [ordered]@{ _meta = $meta } },
        [ordered]@{ jsonrpc = '2.0'; id = 3; method = 'tools/call'; params = [ordered]@{
            name = 'lattice_task_status'
            arguments = [ordered]@{ task_ref = $TaskRef }
            _meta = $meta
        } }
    )
    return ((@($frames | ForEach-Object { $_ | ConvertTo-Json -Compress -Depth 12 }) -join "`n") + "`n")
}

function Get-Task050ExpectedStartupDiagnostics {
    $schema = 'lattice.latticed.startup-diagnostic.v1'
    $records = @(
        @('CONFIGURATION_VALIDATION_STARTED', 'NONE', 'CONFIGURATION_VALIDATION', 'CHECKING', 'NOT_CHECKED', 'NONE'),
        @('CONFIGURATION_VALIDATED', 'CONFIGURATION_VALIDATED', 'SERVICE_ASSEMBLY', 'VALID', 'CONFIGURED_NO_CONNECTIVITY_PROBE', 'NONE'),
        @('SERVICE_ASSEMBLY_STARTED', 'CONFIGURATION_VALIDATED', 'SERVICE_ASSEMBLY', 'VALID', 'ASSEMBLY_IN_PROGRESS', 'NONE'),
        @('SERVICE_ASSEMBLED', 'SERVICE_ASSEMBLED', 'STDIO_ENTRY', 'VALID', 'ASSEMBLED_NO_CONNECTIVITY_PROBE', 'NONE'),
        @('STDIO_LOOP_ENTERED', 'STDIO_LOOP_ENTERED', 'MCP_INPUT', 'VALID', 'MCP_SESSION_PENDING', 'NONE'),
        @('WAITING_FOR_MCP_INPUT', 'STDIO_LOOP_ENTERED', 'MCP_INPUT', 'VALID', 'MCP_SESSION_PENDING', 'NONE'),
        @('MCP_TOOLS_LIST_RECEIVED', 'MCP_TOOLS_LIST_RECEIVED', 'MCP_INPUT', 'VALID', 'MCP_SESSION_ACTIVE', 'NONE'),
        @('MCP_END_OF_STREAM', 'MCP_END_OF_STREAM', 'NONE', 'VALID', 'STDIN_EOF', 'NONE')
    )
    $lines = @($records | ForEach-Object {
        [ordered]@{
            schema = $schema
            stage = [string]$_[0]
            last_completed_stage = [string]$_[1]
            waiting_reason = [string]$_[2]
            configuration_health = [string]$_[3]
            dependency_health = [string]$_[4]
            failure_classification = [string]$_[5]
        } | ConvertTo-Json -Compress
    })
    return (($lines -join "`n") + "`n")
}

function Assert-Task050SafeStartupDiagnostics {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Diagnostics)

    $expected = Get-Task050ExpectedStartupDiagnostics
    if (
        $Diagnostics.Length -gt 16384 -or
        $Diagnostics.Contains("`r") -or
        -not $Diagnostics.Equals($expected, [StringComparison]::Ordinal)
    ) {
        throw 'TASK050_LATTICED_STARTUP_DIAGNOSTIC_REJECTED'
    }
}

function Get-Task050McpResponses {
    param([Parameter(Mandatory = $true)][string]$Output)

    if (
        $Output.Length -gt 1048576 -or $Output.Contains("`r") -or
        -not $Output.EndsWith("`n", [StringComparison]::Ordinal)
    ) {
        throw 'TASK050_MCP_RESPONSE_REJECTED'
    }
    $parts = @($Output.Split([string[]]@("`n"), [StringSplitOptions]::None))
    if ($parts[-1] -cne '') { throw 'TASK050_MCP_RESPONSE_REJECTED' }
    $lines = @($parts[0..($parts.Count - 2)])
    if ($lines.Count -ne 3 -or @($lines | Where-Object { $_ -ceq '' }).Count -ne 0) {
        throw 'TASK050_MCP_RESPONSE_REJECTED'
    }
    $responses = [Collections.Generic.List[object]]::new()
    foreach ($line in $lines) {
        try { $responses.Add(($line | ConvertFrom-Json -ErrorAction Stop)) }
        catch { throw 'TASK050_MCP_RESPONSE_REJECTED' }
    }
    $ids = @($responses | ForEach-Object { [long]$_.id } | Sort-Object)
    if (($ids -join ',') -cne '1,2,3') {
        throw 'TASK050_MCP_RESPONSE_REJECTED'
    }
    return @($responses)
}

function Get-Task050McpResponse {
    param(
        [Parameter(Mandatory = $true)][object[]]$Responses,
        [Parameter(Mandatory = $true)][ValidateRange(1, 3)][int]$Id
    )

    $matches = @($Responses | Where-Object { [long]$_.id -eq $Id })
    if ($matches.Count -ne 1 -or [string]$matches[0].jsonrpc -cne '2.0') {
        throw 'TASK050_MCP_RESPONSE_REJECTED'
    }
    return $matches[0]
}

function New-Task050SessionEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$EvidenceRoot,
        [Parameter(Mandatory = $true)][ValidateSet('initial', 'restart')][string]$Phase,
        [Parameter(Mandatory = $true)][ValidateSet('ASK_USER', 'PROCEED')][string]$Profile,
        [Parameter(Mandatory = $true)][string]$SafeConfigSha256
    )

    $root = [IO.Path]::GetFullPath($EvidenceRoot)
    $targetRoot = [IO.Path]::GetFullPath((Join-Path $repositoryRoot 'target'))
    if (
        -not $root.StartsWith($targetRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
        -not (Test-Path -LiteralPath $root -PathType Container) -or
        ((Get-Item -LiteralPath $root -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK050_SESSION_EVIDENCE_ROOT_REJECTED'
    }
    $sessionId = [Guid]::NewGuid().ToString('N')
    $sessionRoot = Join-Path $root ('.task050-latticed-' + $Phase + '-' + $Profile.ToLowerInvariant() + '-' + $sessionId)
    [IO.Directory]::CreateDirectory($sessionRoot) | Out-Null
    $acceptancePath = Join-Path $sessionRoot 'acceptance.jsonl'
    $effectPath = Join-Path $sessionRoot 'observed-effects.jsonl'
    try {
        foreach ($path in @($acceptancePath, $effectPath)) {
            $stream = [IO.File]::Open($path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
            $stream.Dispose()
        }
    }
    catch {
        foreach ($path in @($acceptancePath, $effectPath)) {
            if (Test-Path -LiteralPath $path) { [IO.File]::Delete($path) }
        }
        if (Test-Path -LiteralPath $sessionRoot) { [IO.Directory]::Delete($sessionRoot, $false) }
        throw 'TASK050_SESSION_EVIDENCE_CREATE_REJECTED'
    }
    return [pscustomobject]@{
        session_id = $sessionId
        safe_config_sha256 = $SafeConfigSha256
        nonce = ([Guid]::NewGuid().ToString('N') + [Guid]::NewGuid().ToString('N'))
        root = $sessionRoot
        acceptance_path = $acceptancePath
        effect_path = $effectPath
    }
}

function Remove-Task050SessionEvidence {
    param([Parameter(Mandatory = $true)]$Evidence)

    foreach ($path in @([string]$Evidence.acceptance_path, [string]$Evidence.effect_path)) {
        if (Test-Path -LiteralPath $path) { [IO.File]::Delete($path) }
        if (Test-Path -LiteralPath $path) { throw 'TASK050_SESSION_EVIDENCE_CLEANUP_REJECTED' }
    }
    if (Test-Path -LiteralPath ([string]$Evidence.root)) {
        [IO.Directory]::Delete([string]$Evidence.root, $false)
    }
    if (Test-Path -LiteralPath ([string]$Evidence.root)) {
        throw 'TASK050_SESSION_EVIDENCE_CLEANUP_REJECTED'
    }
}

function Read-Task050StrictJsonLines {
    param([Parameter(Mandatory = $true)][string]$Path)

    $bytes = [IO.File]::ReadAllBytes($Path)
    if (
        $bytes.Length -lt 1 -or $bytes.Length -gt 1048576 -or
        ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf)
    ) {
        throw 'TASK050_SESSION_EVIDENCE_REJECTED'
    }
    try { $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes) }
    catch { throw 'TASK050_SESSION_EVIDENCE_REJECTED' }
    if (-not $text.EndsWith("`n", [StringComparison]::Ordinal) -or $text.Contains("`r")) {
        throw 'TASK050_SESSION_EVIDENCE_REJECTED'
    }
    $parts = @($text.Split([string[]]@("`n"), [StringSplitOptions]::None))
    if ($parts[-1] -cne '') { throw 'TASK050_SESSION_EVIDENCE_REJECTED' }
    $lines = @($parts[0..($parts.Count - 2)])
    if (@($lines | Where-Object { $_ -ceq '' }).Count -ne 0) {
        throw 'TASK050_SESSION_EVIDENCE_REJECTED'
    }
    $records = [Collections.Generic.List[object]]::new()
    foreach ($line in $lines) {
        try { $records.Add(($line | ConvertFrom-Json -ErrorAction Stop)) }
        catch { throw 'TASK050_SESSION_EVIDENCE_REJECTED' }
    }
    return @($records)
}

function Assert-Task050AcceptanceEvidence {
    param(
        [Parameter(Mandatory = $true)][object[]]$Records,
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)][int]$ProcessId
    )

    $types = @('SESSION_OPEN', 'DISPATCH_ACCEPTED', 'SESSION_CLOSED')
    if ($Records.Count -ne $types.Count) { throw 'TASK050_ACCEPTANCE_EVIDENCE_REJECTED' }
    $previous = '0' * 64
    for ($index = 0; $index -lt $Records.Count; $index++) {
        $record = $Records[$index]
        $expectedKeys = @(
            'dispatch_accepted_count', 'event_sha256', 'observed_at_unix_nanos', 'ordinal',
            'previous_event_sha256', 'process_id', 'record_type', 'request_id_sha256',
            'safe_config_sha256', 'schema', 'session_id', 'tool_name'
        ) | Sort-Object -CaseSensitive
        $actualKeys = @($record.PSObject.Properties.Name | Sort-Object -CaseSensitive)
        $dispatchCount = if ($index -eq 0) { 0 } else { 1 }
        $toolName = if ($index -eq 1) { 'lattice_task_status' } else { 'null' }
        $requestId = if ($index -eq 1) { [string]$record.request_id_sha256 } else { 'null' }
        if (
            ($actualKeys -join "`n") -cne ($expectedKeys -join "`n") -or
            [string]$record.schema -cne 'lattice.mcp.acceptance-dispatch.v1' -or
            [string]$record.record_type -cne $types[$index] -or
            [string]$record.session_id -cne [string]$Evidence.session_id -or
            [string]$record.safe_config_sha256 -cne [string]$Evidence.safe_config_sha256 -or
            [long]$record.process_id -ne $ProcessId -or
            [long]$record.ordinal -ne ($index + 1) -or
            [long]$record.dispatch_accepted_count -ne $dispatchCount -or
            [string]$record.observed_at_unix_nanos -cnotmatch '\A[1-9][0-9]*\z' -or
            [string]$record.previous_event_sha256 -cne $previous -or
            [string]$record.event_sha256 -cnotmatch '\A[0-9a-f]{64}\z'
        ) { throw 'TASK050_ACCEPTANCE_EVIDENCE_REJECTED' }
        if ($index -eq 1) {
            if (
                [string]$record.tool_name -cne $toolName -or
                $requestId -cnotmatch '\A[0-9a-f]{64}\z'
            ) { throw 'TASK050_ACCEPTANCE_EVIDENCE_REJECTED' }
        }
        elseif ($null -ne $record.tool_name -or $null -ne $record.request_id_sha256) {
            throw 'TASK050_ACCEPTANCE_EVIDENCE_REJECTED'
        }
        $hashInput = @(
            'lattice.mcp.acceptance-dispatch-hash.v1', $previous, [string]$Evidence.session_id,
            [string]$Evidence.safe_config_sha256, $types[$index], [string]($index + 1),
            [string]$ProcessId, $toolName, $requestId, [string]$dispatchCount,
            [string]$record.observed_at_unix_nanos
        ) -join "`n"
        if ([string]$record.event_sha256 -cne (Get-Task050StringSha256 -Value $hashInput)) {
            throw 'TASK050_ACCEPTANCE_EVIDENCE_REJECTED'
        }
        $previous = [string]$record.event_sha256
    }
}

function Assert-Task050ObservedEffectRecords {
    param(
        [Parameter(Mandatory = $true)][object[]]$Records,
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)][int]$ProcessId
    )

    $types = @(
        'SESSION_OPEN', 'PROBE_OPEN', 'DISPATCH_ACCEPTED', 'EFFECT_OBSERVED',
        'EFFECT_OBSERVED', 'PROBE_COMPLETED', 'SESSION_CLOSED'
    )
    if ($Records.Count -ne $types.Count) { throw 'TASK050_OBSERVED_EFFECT_EVIDENCE_REJECTED' }
    $nonceCommitment = Get-Task050StringSha256 -Value (@(
        'lattice.mcp.observed-effect-nonce.v1', [string]$Evidence.session_id,
        [string]$Evidence.safe_config_sha256, [string]$Evidence.nonce
    ) -join "`n")
    $previous = '0' * 64
    $probeId = $null
    $requestId = $null
    for ($index = 0; $index -lt $Records.Count; $index++) {
        $record = $Records[$index]
        $expectedKeys = @(
            'classification', 'effect_kind', 'event_sha256', 'nonce_commitment',
            'observed_at_unix_nanos', 'ordinal', 'previous_event_sha256', 'probe_counters',
            'probe_id', 'process_id', 'record_type', 'request_id_sha256', 'safe_config_sha256',
            'schema', 'session_counters', 'session_id', 'tool_name'
        ) | Sort-Object -CaseSensitive
        $actualKeys = @($record.PSObject.Properties.Name | Sort-Object -CaseSensitive)
        if (
            ($actualKeys -join "`n") -cne ($expectedKeys -join "`n") -or
            [string]$record.schema -cne 'lattice.mcp.observed-effect.v1' -or
            [string]$record.record_type -cne $types[$index] -or
            [string]$record.session_id -cne [string]$Evidence.session_id -or
            [string]$record.safe_config_sha256 -cne [string]$Evidence.safe_config_sha256 -or
            [string]$record.nonce_commitment -cne $nonceCommitment -or
            [long]$record.process_id -ne $ProcessId -or
            [long]$record.ordinal -ne ($index + 1) -or
            [string]$record.observed_at_unix_nanos -cnotmatch '\A[1-9][0-9]*\z' -or
            [string]$record.previous_event_sha256 -cne $previous -or
            [string]$record.event_sha256 -cnotmatch '\A[0-9a-f]{64}\z'
        ) { throw 'TASK050_OBSERVED_EFFECT_EVIDENCE_REJECTED' }
        if ($index -eq 1) {
            $probeId = [string]$record.probe_id
            $requestId = [string]$record.request_id_sha256
            if ($probeId -cnotmatch '\A[0-9a-f]{64}\z' -or $requestId -cnotmatch '\A[0-9a-f]{64}\z') {
                throw 'TASK050_OBSERVED_EFFECT_EVIDENCE_REJECTED'
            }
        }
        $probeBound = $index -ge 1 -and $index -le 5
        if ($probeBound) {
            if (
                [string]$record.probe_id -cne $probeId -or
                [string]$record.tool_name -cne 'lattice_task_status' -or
                [string]$record.request_id_sha256 -cne $requestId
            ) { throw 'TASK050_OBSERVED_EFFECT_EVIDENCE_REJECTED' }
        }
        elseif ($null -ne $record.probe_id -or $null -ne $record.tool_name -or $null -ne $record.request_id_sha256) {
            throw 'TASK050_OBSERVED_EFFECT_EVIDENCE_REJECTED'
        }
        $expectedClassification = switch ($index) {
            2 { 'MCP_DISPATCH_ACCEPTED' }
            5 { 'MCP_RESULT' }
            default { $null }
        }
        $expectedEffect = switch ($index) {
            3 { 'database' }
            4 { 'network' }
            default { $null }
        }
        if (
            $record.classification -cne $expectedClassification -or
            $record.effect_kind -cne $expectedEffect
        ) { throw 'TASK050_OBSERVED_EFFECT_EVIDENCE_REJECTED' }
        $counterField = {
            param($counter)
            return (@('dispatch','database','filesystem','process','network','codex') |
                ForEach-Object { [string][long]$counter.$_ }) -join ':'
        }
        $expectedProbeField = @(
            '0:0:0:0:0:0', '0:0:0:0:0:0', '1:0:0:0:0:0', '1:1:0:0:0:0',
            '1:1:0:0:1:0', '1:1:0:0:1:0', '0:0:0:0:0:0'
        )[$index]
        $expectedSessionField = @(
            '0:0:0:0:0:0', '0:0:0:0:0:0', '1:0:0:0:0:0', '1:1:0:0:0:0',
            '1:1:0:0:1:0', '1:1:0:0:1:0', '1:1:0:0:1:0'
        )[$index]
        if (
            (& $counterField $record.probe_counters) -cne $expectedProbeField -or
            (& $counterField $record.session_counters) -cne $expectedSessionField
        ) { throw 'TASK050_OBSERVED_EFFECT_EVIDENCE_REJECTED' }
        $hashInput = @(
            'lattice.mcp.observed-effect-hash.v1', $previous, [string]$Evidence.session_id,
            [string]$Evidence.safe_config_sha256, $nonceCommitment, [string]($index + 1),
            $types[$index], $(if ($probeBound) { $probeId } else { 'null' }),
            $(if ($probeBound) { 'lattice_task_status' } else { 'null' }),
            $(if ($probeBound) { $requestId } else { 'null' }),
            $(if ($null -eq $expectedClassification) { 'null' } else { $expectedClassification }),
            $(if ($null -eq $expectedEffect) { 'null' } else { $expectedEffect }),
            (& $counterField $record.probe_counters), (& $counterField $record.session_counters),
            [string]$record.observed_at_unix_nanos
        ) -join "`n"
        if ([string]$record.event_sha256 -cne (Get-Task050HmacSha256 -Key ([string]$Evidence.nonce) -Value $hashInput)) {
            throw 'TASK050_OBSERVED_EFFECT_EVIDENCE_REJECTED'
        }
        $previous = [string]$record.event_sha256
    }
    Assert-Task050ObservedEffectEvidence -Evidence $Records[-1]
}

function Get-Task050ProhibitedProcessFootprint {
    return @(
        Get-Process -ErrorAction Stop |
            Where-Object { $_.ProcessName -cmatch '\A(?:codex|git|gh|hermes|openclaw|latticed)\z' } |
            ForEach-Object { ([string]$_.Id + ':' + [string]$_.ProcessName) } |
            Sort-Object -CaseSensitive
    )
}

function Assert-Task050NoDescendantOrProcessFootprint {
    param(
        [Parameter(Mandatory = $true)][string[]]$Before,
        [Parameter(Mandatory = $true)][int]$SessionProcessId
    )

    if ($null -ne (Get-Process -Id $SessionProcessId -ErrorAction SilentlyContinue)) {
        throw 'TASK050_LATTICED_DESCENDANT_REJECTED'
    }
    $after = @(Get-Task050ProhibitedProcessFootprint)
    if (@(Compare-Object -ReferenceObject $Before -DifferenceObject $after -CaseSensitive).Count -ne 0) {
        throw 'TASK050_PROHIBITED_PROCESS_FOOTPRINT_REJECTED'
    }
}

function Assert-Task050FourToolDiscovery {
    param(
        [Parameter(Mandatory = $true)][object[]]$Tools
    )

    $names = @($Tools | ForEach-Object { [string]$_.name } | Sort-Object -CaseSensitive)
    $expected = @($script:Task050ExpectedTools | Sort-Object -CaseSensitive)
    if (
        $Tools.Count -ne 4 -or
        ($names -join "`n") -cne ($expected -join "`n")
    ) {
        throw 'TASK050_FOUR_TOOL_DISCOVERY_REJECTED'
    }
}

function Assert-Task050SixFieldStatus {
    param(
        [Parameter(Mandatory = $true)]$Status
    )

    if ($null -eq $Status) { throw 'TASK050_SIX_FIELD_STATUS_REJECTED' }
    $actual = @($Status.PSObject.Properties.Name | Sort-Object -CaseSensitive)
    $expected = @($script:Task050PublicStatusFields | Sort-Object -CaseSensitive)
    if (
        $actual.Count -ne 6 -or
        ($actual -join "`n") -cne ($expected -join "`n") -or
        [string]$Status.schema_version -cne 'lattice.task.status.v1' -or
        [string]$Status.status -cnotmatch '\A(?:NOT_SUBMITTED|RECONCILIATION_REQUIRED|FAILED|COMPLETED)\z' -or
        [string]$Status.task_state -cnotmatch '\A(?:NOT_SUBMITTED|DRAFT|AWAITING_EXECUTION_APPROVAL|PREPARING|EXECUTING|VERIFYING|REVIEWING|AWAITING_MERGE_APPROVAL|MERGING|COMPLETED|REJECTED|BLOCKED|FAILED|STOPPING|CANCELLED)\z' -or
        [string]$Status.task_ref -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$Status.ledger_head_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        ($null -ne $Status.result_digest -and [string]$Status.result_digest -cnotmatch '\A[0-9a-f]{64}\z')
    ) {
        throw 'TASK050_SIX_FIELD_STATUS_REJECTED'
    }
}

function Assert-Task050ObservedEffectEvidence {
    param(
        [Parameter(Mandatory = $true)]$Evidence
    )

    $counters = $Evidence.session_counters
    if (
        $null -eq $counters -or
        [long]$counters.dispatch -ne 1 -or
        [long]$counters.database -ne 1 -or
        [long]$counters.network -ne 1 -or
        [long]$counters.filesystem -ne 0 -or
        [long]$counters.process -ne 0 -or
        [long]$counters.codex -ne 0
    ) {
        throw 'TASK050_OBSERVED_EFFECT_EVIDENCE_REJECTED'
    }
}

function Assert-Task050SamePublicStatus {
    param(
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)]$Actual
    )

    Assert-Task050SixFieldStatus -Status $Expected
    Assert-Task050SixFieldStatus -Status $Actual
    foreach ($name in $script:Task050PublicStatusFields) {
        $expectedValue = $Expected.$name | ConvertTo-Json -Compress -Depth 4
        $actualValue = $Actual.$name | ConvertTo-Json -Compress -Depth 4
        if ([string]$expectedValue -cne [string]$actualValue) {
            throw 'TASK050_PUBLIC_STATUS_REPLAY_REJECTED'
        }
    }
}

function Invoke-Task050CanonicalLatticedSession {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('initial', 'restart')][string]$Phase,
        [Parameter(Mandatory = $true)][ValidateSet('ASK_USER', 'PROCEED')][string]$Profile,
        [Parameter(Mandatory = $true)]$ProfileInput,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot
    )

    $latticed = Resolve-Task050LatticedExecutable
    $startInfo = New-Task050LatticedProcessStartInfo -LatticedExecutable $latticed
    if (
        [string]$ProfileInput.database_run_id -cne
            [string]$startInfo.EnvironmentVariables['LATTICE_TASK019_RUN_ID'] -or
        [string]$ProfileInput.task_spec_digest -cne
            [string]$script:Task050ProfileTaskSpecDigests[$Profile]
    ) {
        throw 'TASK050_LATTICED_PROFILE_INPUT_REJECTED'
    }
    foreach ($required in @(
        'LATTICE_TASK019_HOST', 'LATTICE_TASK019_PORT',
        'LATTICE_TASK019_RUN_ID', 'LATTICE_TASK019_PASSWORD'
    )) {
        if ([string]::IsNullOrEmpty([string]$startInfo.EnvironmentVariables[$required])) {
            throw 'TASK050_LATTICED_DATABASE_INPUT_REJECTED'
        }
    }
    $databasePort = 0
    if (
        [string]$startInfo.EnvironmentVariables['LATTICE_TASK019_HOST'] -cne '127.0.0.1' -or
        -not [int]::TryParse(
            [string]$startInfo.EnvironmentVariables['LATTICE_TASK019_PORT'],
            [ref]$databasePort
        ) -or
        $databasePort -lt 1 -or $databasePort -gt 65535 -or
        [string]$startInfo.EnvironmentVariables['LATTICE_TASK019_RUN_ID'] -cnotmatch '\A[0-9a-f]{32}\z'
    ) {
        throw 'TASK050_LATTICED_DATABASE_INPUT_REJECTED'
    }
    $inputText = New-Task050McpInput -TaskRef ([string]$ProfileInput.task_ref)
    $latticedSha256 = (Get-FileHash -LiteralPath $latticed -Algorithm SHA256).Hash.ToLowerInvariant()
    $safeConfigSha256 = Get-Task050StringSha256 -Value (@(
        'lattice.task050.latticed-session-safe-config.v1', $Phase, $Profile,
        [string]$ProfileInput.database_run_id, [string]$ProfileInput.task_spec_digest,
        [string]$ProfileInput.task_ref, [string]$ProfileInput.ingress_profile_sha256,
        $latticedSha256, (Get-Task050StringSha256 -Value $inputText)
    ) -join "`n")
    $evidence = New-Task050SessionEvidence `
        -EvidenceRoot $EvidenceRoot `
        -Phase $Phase `
        -Profile $Profile `
        -SafeConfigSha256 $safeConfigSha256
    $process = $null
    $started = $false
    $processId = 0
    $beforeProcesses = @()
    try {
    foreach ($name in @($startInfo.EnvironmentVariables.Keys)) {
        if (
            [string]$name -cmatch '\A(?:CODEX|GH|GITHUB)_' -or
            [string]$name -cmatch '\ALATTICE_(?:HERMES|OPENCLAW|MCP_ACCEPTANCE|MCP_OBSERVED_EFFECT)' -or
            [string]$name -cmatch '\ALATTICE_TASK050_ACCEPTANCE_' -or
            [string]$name -cmatch '\ALATTICE_DELIVERY_(?:LAUNCHER|LAUNCHER_VERSION|LAUNCHER_SHA256|SCHEMA_DIR|CODEX_HOME|ROOT|GIT_EXE)\z'
        ) {
            $startInfo.EnvironmentVariables.Remove([string]$name)
        }
    }
    $environment = [ordered]@{
        LATTICE_FULL_CHAIN_RUN_MODE = 'RESUME_EXISTING'
        LATTICE_TASK_INGRESS_KIND = 'LOCAL_CANONICAL_MCP_ACCEPTANCE'
        LATTICE_TASK_INGRESS_PROFILE_SHA256 = [string]$ProfileInput.ingress_profile_sha256
        LATTICE_TASK050_ACCEPTANCE_PROFILE = $Profile
        LATTICE_TASK050_ACCEPTANCE_TASK_SPEC_SHA256 = [string]$ProfileInput.task_spec_digest
        LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
        LATTICE_DELIVERY_TIMEOUT_SECONDS = '30'
        LATTICE_STORE_DAEMON_INSTANCE_ID = [string]$ProfileInput.daemon_instance_id
        LATTICE_STORE_DAEMON_EPOCH = [string][long]$ProfileInput.daemon_epoch
        LATTICE_STORE_AUTHORITY_REVISION = [string][long]$ProfileInput.authority_revision
        LATTICE_STORE_OBSERVATION_DIGEST = [string]$ProfileInput.observation_digest
        LATTICE_STORE_AUTHORITY_HEAD_DIGEST = [string]$ProfileInput.authority_head_digest
        LATTICE_MCP_ACCEPTANCE_EVIDENCE_PATH = [string]$evidence.acceptance_path
        LATTICE_MCP_ACCEPTANCE_SESSION_ID = [string]$evidence.session_id
        LATTICE_MCP_ACCEPTANCE_SAFE_CONFIG_SHA256 = [string]$evidence.safe_config_sha256
        LATTICE_MCP_OBSERVED_EFFECT_PATH = [string]$evidence.effect_path
        LATTICE_MCP_OBSERVED_EFFECT_NONCE = [string]$evidence.nonce
    }
    foreach ($entry in $environment.GetEnumerator()) {
        $startInfo.EnvironmentVariables[[string]$entry.Key] = [string]$entry.Value
    }
    $beforeProcesses = @(Get-Task050ProhibitedProcessFootprint)
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
        if (-not $process.Start()) { throw 'TASK050_LATTICED_START_REJECTED' }
        $started = $true
        $processId = [int]$process.Id
        if ($processId -le 0 -or -not $script:Task050ObservedProcessIds.Add($processId)) {
            throw 'TASK050_LATTICED_FRESH_PID_REJECTED'
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.StandardInput.Write($inputText)
        $process.StandardInput.Close()
        if (-not $process.WaitForExit(60000)) { throw 'TASK050_LATTICED_TIMEOUT' }
        if (-not $stdoutTask.Wait(5000) -or -not $stderrTask.Wait(5000)) {
            throw 'TASK050_LATTICED_OUTPUT_REJECTED'
        }
        $stdout = [string]$stdoutTask.Result
        $stderr = [string]$stderrTask.Result
        if ($process.ExitCode -ne 0) {
            throw ('TASK050_LATTICED_SESSION_REJECTED|' + (Get-Task050StringSha256 -Value $stderr))
        }
        Assert-Task050SafeStartupDiagnostics -Diagnostics $stderr
        $responses = @(Get-Task050McpResponses -Output $stdout)
        $discover = Get-Task050McpResponse -Responses $responses -Id 1
        $tools = Get-Task050McpResponse -Responses $responses -Id 2
        $statusResponse = Get-Task050McpResponse -Responses $responses -Id 3
        if (
            $null -ne $discover.PSObject.Properties['error'] -or
            $null -ne $tools.PSObject.Properties['error'] -or
            $null -ne $statusResponse.PSObject.Properties['error'] -or
            $null -eq $tools.result -or
            $null -eq $statusResponse.result -or
            [bool]$statusResponse.result.isError
        ) { throw 'TASK050_MCP_RESPONSE_REJECTED' }
        Assert-Task050FourToolDiscovery -Tools @($tools.result.tools)
        $status = $statusResponse.result.structuredContent
        Assert-Task050SamePublicStatus -Expected $ProfileInput.expected_status -Actual $status
        $content = @($statusResponse.result.content)
        if ($content.Count -ne 1 -or [string]$content[0].type -cne 'text') {
            throw 'TASK050_SIX_FIELD_STATUS_REJECTED'
        }
        try { $textStatus = [string]$content[0].text | ConvertFrom-Json -ErrorAction Stop }
        catch { throw 'TASK050_SIX_FIELD_STATUS_REJECTED' }
        Assert-Task050SamePublicStatus -Expected $status -Actual $textStatus
        if ($stdout -cmatch '(?:autonomy_receipt|authority_digest|writer_lease_receipt_digest|writer_fencing_token)') {
            throw 'TASK050_INTERNAL_RECEIPT_WIRE_LEAK_REJECTED'
        }
        Assert-Task050AcceptanceEvidence `
            -Records @(Read-Task050StrictJsonLines -Path ([string]$evidence.acceptance_path)) `
            -Evidence $evidence `
            -ProcessId $processId
        Assert-Task050ObservedEffectRecords `
            -Records @(Read-Task050StrictJsonLines -Path ([string]$evidence.effect_path)) `
            -Evidence $evidence `
            -ProcessId $processId
        return [pscustomobject]@{ Phase = $Phase; Profile = $Profile; ProcessId = $processId }
    }
    finally {
        $cleanupFailure = $null
        try {
            if ($null -ne $process) {
                if ($started -and -not $process.HasExited) {
                    $process.Kill()
                    $process.WaitForExit(5000) | Out-Null
                }
                $process.Dispose()
            }
        }
        catch {
            $cleanupFailure = $_
        }
        try {
            if ($processId -gt 0) {
                Assert-Task050NoDescendantOrProcessFootprint `
                    -Before $beforeProcesses `
                    -SessionProcessId $processId
            }
        }
        catch {
            if ($null -eq $cleanupFailure) { $cleanupFailure = $_ }
        }
        try { Remove-Task050SessionEvidence -Evidence $evidence }
        catch {
            if ($null -eq $cleanupFailure) { $cleanupFailure = $_ }
        }
        $evidence.nonce = $null
        if ($null -ne $cleanupFailure) { throw $cleanupFailure }
    }
}

function Invoke-Task050RuntimeProfileFixture {
    param(
        [Parameter(Mandatory = $true)][string]$Cargo,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][ValidateSet('initial', 'restart')][string]$Phase,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot
    )

    if (
        [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PHASE', 'Process') -cne $Phase -or
        [Environment]::GetEnvironmentVariable('LATTICE_TASK019_RUN_ID', 'Process') -cnotmatch
            '\A[0-9a-f]{32}\z'
    ) {
        throw 'TASK050_RUNTIME_PROFILE_FIXTURE_INPUT_REJECTED'
    }
    $stdoutPath = Join-Path $EvidenceRoot ".cargo-task050-profile-$Phase-stdout.log"
    $stderrPath = Join-Path $EvidenceRoot ".cargo-task050-profile-$Phase-stderr.log"
    $process = $null
    $exitCode = $null
    $output = @()
    Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    try {
        $process = Start-Process -FilePath $Cargo -ArgumentList @(
            'test', '--locked', '-p', 'lattice-runtime', '--lib',
            'composition::tests::task050_canonical_latticed_profiles_when_provisioned',
            '--', '--ignored', '--exact', '--nocapture', '--test-threads=1'
        ) -WorkingDirectory $RepositoryRoot -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath -WindowStyle Hidden -PassThru
        $null = $process.Handle
        $process.WaitForExit()
        $exitCode = [int]$process.ExitCode
        if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) {
            $output += @(Get-Content -LiteralPath $stdoutPath -Encoding utf8)
        }
        if (Test-Path -LiteralPath $stderrPath -PathType Leaf) {
            $output += @(Get-Content -LiteralPath $stderrPath -Encoding utf8)
        }
    }
    finally {
        if ($null -ne $process) { $process.Dispose() }
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $path) {
                throw 'TASK050_RUNTIME_PROFILE_FIXTURE_OUTPUT_DELETE_FAILED'
            }
        }
    }
    if (
        $exitCode -ne 0 -or
        @($output | Where-Object {
            ([string]$_).StartsWith(
                $script:Task050ProfileMarkerPrefix,
                [StringComparison]::Ordinal
            )
        }).Count -ne 2
    ) {
        throw 'TASK050_RUNTIME_PROFILE_FIXTURE_REJECTED'
    }
    return ,$output
}

function Get-Task050ProceedWriterMatrixAuthorityProfile {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$TestOutput
    )

    $profileInputs = @(Get-Task050PhaseProfileInputs -Phase 'restart' -TestOutput $TestOutput)
    $authorityProfiles = @($profileInputs | ForEach-Object {
        @(
            [string]$_.daemon_instance_id,
            [string][long]$_.daemon_epoch,
            [string][long]$_.authority_revision,
            [string]$_.observation_digest,
            [string]$_.authority_head_digest
        ) -join "`n"
    } | Sort-Object -CaseSensitive -Unique)
    if ($profileInputs.Count -ne 2 -or $authorityProfiles.Count -ne 1) {
        throw 'TASK050_PROCEED_WRITER_MATRIX_AUTHORITY_REJECTED'
    }
    return $profileInputs[0]
}

function New-Task050ProceedWriterMatrixProcessStartInfo {
    param(
        [Parameter(Mandatory = $true)][string]$Cargo,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)]$AuthorityProfile
    )

    if (
        [string]$AuthorityProfile.daemon_instance_id -cnotmatch '\A[a-z0-9][a-z0-9-]{0,63}\z' -or
        [long]$AuthorityProfile.daemon_epoch -le 0 -or
        [long]$AuthorityProfile.authority_revision -le 0 -or
        [string]$AuthorityProfile.observation_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$AuthorityProfile.authority_head_digest -cnotmatch '\A[0-9a-f]{64}\z'
    ) {
        throw 'TASK050_PROCEED_WRITER_MATRIX_AUTHORITY_REJECTED'
    }
    $arguments = @(
        'test', '--locked', '-p', 'lattice-runtime', '--test', 'task_control',
        'task050_proceed_requires_current_writer_and_retries_without_currentness_when_provisioned',
        '--', '--exact', '--nocapture', '--test-threads=1'
    )
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Cargo
    $startInfo.Arguments = $arguments -join ' '
    $startInfo.WorkingDirectory = $RepositoryRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $startInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    $startInfo.EnvironmentVariables['LATTICE_STORE_DAEMON_INSTANCE_ID'] =
        [string]$AuthorityProfile.daemon_instance_id
    $startInfo.EnvironmentVariables['LATTICE_STORE_DAEMON_EPOCH'] =
        [string][long]$AuthorityProfile.daemon_epoch
    $startInfo.EnvironmentVariables['LATTICE_STORE_AUTHORITY_REVISION'] =
        [string][long]$AuthorityProfile.authority_revision
    $startInfo.EnvironmentVariables['LATTICE_STORE_OBSERVATION_DIGEST'] =
        [string]$AuthorityProfile.observation_digest
    $startInfo.EnvironmentVariables['LATTICE_STORE_AUTHORITY_HEAD_DIGEST'] =
        [string]$AuthorityProfile.authority_head_digest
    return $startInfo
}

function Assert-Task050ProceedWriterMatrixOutput {
    param(
        [Parameter(Mandatory = $true)][int]$ExitCode,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Output
    )

    $testName =
        'task050_proceed_requires_current_writer_and_retries_without_currentness_when_provisioned'
    $expectedMarker =
        'TASK050_POSTGRES_PROCEED_WRITER_MATRIX_OK current=1 stale=1 wrong_fence=1 substituted=1 exact_retry=1'
    $knownPrefix = 'test ' + $testName + ' ... '
    $markerPattern = '(?m)^(?:' +
        [regex]::Escape($expectedMarker) + '|' +
        [regex]::Escape($knownPrefix + $expectedMarker) + ')\r?$'
    $markerPrefixPattern = '(?m)^(?:' +
        [regex]::Escape('TASK050_POSTGRES_PROCEED_WRITER_MATRIX_OK') + '|' +
        [regex]::Escape($knownPrefix + 'TASK050_POSTGRES_PROCEED_WRITER_MATRIX_OK') +
        ')[^\r\n]*\r?$'
    if (
        $ExitCode -ne 0 -or
        $Output.Length -gt 1048576 -or
        [regex]::Matches($Output, $markerPattern).Count -ne 1 -or
        [regex]::Matches($Output, $markerPrefixPattern).Count -ne 1
    ) {
        throw 'TASK050_PROCEED_WRITER_MATRIX_REJECTED'
    }
}

function Invoke-Task050ProceedWriterMatrix {
    param(
        [Parameter(Mandatory = $true)][string]$Cargo,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$TestOutput
    )

    $expectedMarker = 'TASK050_POSTGRES_PROCEED_WRITER_MATRIX_OK current=1 stale=1 wrong_fence=1 substituted=1 exact_retry=1'
    $cargoIsFullyQualified = try {
        [IO.Path]::GetFullPath($Cargo) -ceq $Cargo
    }
    catch {
        $false
    }
    if (
        -not $cargoIsFullyQualified -or
        [IO.Path]::GetFileName($Cargo) -cne 'cargo.exe' -or
        -not (Test-Path -LiteralPath $Cargo -PathType Leaf) -or
        [Environment]::GetEnvironmentVariable('LATTICE_TASK050_LIVE', 'Process') -cne '1' -or
        [Environment]::GetEnvironmentVariable('LATTICE_TASK019_LIVE', 'Process') -cne '1' -or
        [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PHASE', 'Process') -cne 'restart' -or
        [Environment]::GetEnvironmentVariable('LATTICE_TASK019_HOST', 'Process') -cne '127.0.0.1' -or
        [Environment]::GetEnvironmentVariable('LATTICE_TASK019_RUN_ID', 'Process') -cnotmatch
            '\A[0-9a-f]{32}\z'
    ) {
        throw 'TASK050_PROCEED_WRITER_MATRIX_INPUT_REJECTED'
    }
    $authorityProfile = Get-Task050ProceedWriterMatrixAuthorityProfile -TestOutput $TestOutput
    $resolvedRepositoryRoot = [IO.Path]::GetFullPath(
        (Resolve-Path -LiteralPath $RepositoryRoot -ErrorAction Stop).ProviderPath
    )
    $resolvedEvidenceRoot = [IO.Path]::GetFullPath(
        (Resolve-Path -LiteralPath $EvidenceRoot -ErrorAction Stop).ProviderPath
    )
    if (
        -not (Test-Path -LiteralPath $resolvedRepositoryRoot -PathType Container) -or
        -not (Test-Path -LiteralPath $resolvedEvidenceRoot -PathType Container)
    ) {
        throw 'TASK050_PROCEED_WRITER_MATRIX_INPUT_REJECTED'
    }
    $stdoutPath = [IO.Path]::GetFullPath(
        (Join-Path $resolvedEvidenceRoot '.cargo-task050-proceed-writer-matrix-stdout.log')
    )
    $stderrPath = [IO.Path]::GetFullPath(
        (Join-Path $resolvedEvidenceRoot '.cargo-task050-proceed-writer-matrix-stderr.log')
    )
    foreach ($path in @($stdoutPath, $stderrPath)) {
        if (-not [string]::Equals(
                [IO.Path]::GetDirectoryName($path),
                $resolvedEvidenceRoot,
                [StringComparison]::OrdinalIgnoreCase
            )) {
            throw 'TASK050_PROCEED_WRITER_MATRIX_OUTPUT_PATH_REJECTED'
        }
        Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
    }

    $process = $null
    $exitCode = $null
    $output = @()
    $combinedOutput = ''
    $executionFailed = $false
    try {
        $startInfo = New-Task050ProceedWriterMatrixProcessStartInfo `
            -Cargo $Cargo `
            -RepositoryRoot $resolvedRepositoryRoot `
            -AuthorityProfile $authorityProfile
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        if (-not $process.Start()) {
            throw 'TASK050_PROCEED_WRITER_MATRIX_START_REJECTED'
        }
        $null = $process.Handle
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        if (-not $stdoutTask.Wait(5000) -or -not $stderrTask.Wait(5000)) {
            throw 'TASK050_PROCEED_WRITER_MATRIX_OUTPUT_REJECTED'
        }
        $exitCode = [int]$process.ExitCode
        $stdout = [string]$stdoutTask.Result
        $stderr = [string]$stderrTask.Result
        $combinedOutput = $stdout + $stderr
        $utf8 = [Text.UTF8Encoding]::new($false)
        [IO.File]::WriteAllText($stdoutPath, $stdout, $utf8)
        [IO.File]::WriteAllText($stderrPath, $stderr, $utf8)
        $output += @($stdout, $stderr)
    }
    catch {
        $executionFailed = $true
    }
    finally {
        if ($null -ne $process) {
            try {
                if (-not $process.HasExited) {
                    $process.Kill()
                    $process.WaitForExit(5000) | Out-Null
                }
            }
            catch {
                $executionFailed = $true
            }
            $process.Dispose()
        }
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $path) {
                throw 'TASK050_PROCEED_WRITER_MATRIX_OUTPUT_DELETE_FAILED'
            }
        }
    }
    if ($executionFailed) {
        throw 'TASK050_PROCEED_WRITER_MATRIX_REJECTED'
    }
    Assert-Task050ProceedWriterMatrixOutput -ExitCode $exitCode -Output $combinedOutput
}

function Invoke-Task050CanonicalLatticedPhase {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('initial', 'restart')][string]$Phase,
        [Parameter(Mandatory = $true)][object[]]$TestOutput,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot
    )

    $expectedBefore = if ($Phase -ceq 'initial') { 0 } else { 2 }
    if ($script:Task050ObservedProcessIds.Count -ne $expectedBefore) {
        throw 'TASK050_LATTICED_PHASE_ORDER_REJECTED'
    }
    $inputs = @(Get-Task050PhaseProfileInputs -Phase $Phase -TestOutput $TestOutput)
    foreach ($input in $inputs) {
        $null = Invoke-Task050CanonicalLatticedSession `
            -Phase $Phase `
            -Profile ([string]$input.profile) `
            -ProfileInput $input `
            -EvidenceRoot $EvidenceRoot
    }
    if ($script:Task050ObservedProcessIds.Count -ne ($expectedBefore + 2)) {
        throw 'TASK050_LATTICED_FRESH_PID_REJECTED'
    }
}

function Assert-Task050CanonicalLatticedRunnerShape {
    param(
        [Parameter(Mandatory = $true)][string]$TransformedSource
    )

    if (
        ($script:Task050CanonicalPhases -join ',') -cne 'initial,restart' -or
        ($script:Task050AutonomyProfiles -join ',') -cne 'ASK_USER,PROCEED' -or
        $script:Task050ExpectedTools.Count -ne 4 -or
        $script:Task050PublicStatusFields.Count -ne 6 -or
        -not $script:Task050CanonicalExecutionImplemented
    ) {
        throw 'TASK050_CANONICAL_LATTICED_RUNNER_SHAPE_REJECTED'
    }

    $tokens = $null
    $parseErrors = $null
    $runnerAst = [Management.Automation.Language.Parser]::ParseFile(
        $PSCommandPath,
        [ref]$tokens,
        [ref]$parseErrors
    )
    if (@($parseErrors).Count -ne 0) {
        throw 'TASK050_CANONICAL_LATTICED_RUNNER_SHAPE_REJECTED'
    }
    $requiredFunctions = @(
        'Resolve-Task050LatticedExecutable',
        'New-Task050LatticedProcessStartInfo',
        'Invoke-Task050RuntimeProfileFixture',
        'Get-Task050ProceedWriterMatrixAuthorityProfile',
        'New-Task050ProceedWriterMatrixProcessStartInfo',
        'Assert-Task050ProceedWriterMatrixOutput',
        'Invoke-Task050ProceedWriterMatrix',
        'Invoke-Task050CanonicalLatticedPhase',
        'Invoke-Task050CanonicalLatticedSession',
        'Get-Task050PhaseProfileInputs',
        'New-Task050McpInput',
        'Get-Task050ExpectedStartupDiagnostics',
        'Assert-Task050SafeStartupDiagnostics',
        'Get-Task050McpResponses',
        'New-Task050SessionEvidence',
        'Assert-Task050AcceptanceEvidence',
        'Assert-Task050ObservedEffectRecords',
        'Get-Task050ProhibitedProcessFootprint',
        'Assert-Task050NoDescendantOrProcessFootprint',
        'Assert-Task050FourToolDiscovery',
        'Assert-Task050SixFieldStatus',
        'Assert-Task050ObservedEffectEvidence'
    )
    foreach ($name in $requiredFunctions) {
        $matches = @($runnerAst.FindAll({
            param($node)
            $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
            $node.Name -ceq $name
        }, $true))
        if ($matches.Count -ne 1) {
            throw 'TASK050_CANONICAL_LATTICED_RUNNER_SHAPE_REJECTED'
        }
    }

    $shapeStartInfo = New-Task050LatticedProcessStartInfo `
        -LatticedExecutable 'C:\task050-shape\latticed.exe'
    if (
        [string]$shapeStartInfo.FileName -cne 'C:\task050-shape\latticed.exe' -or
        $shapeStartInfo.UseShellExecute -or
        -not $shapeStartInfo.RedirectStandardInput -or
        -not $shapeStartInfo.RedirectStandardOutput -or
        -not $shapeStartInfo.RedirectStandardError
    ) {
        throw 'TASK050_CANONICAL_LATTICED_RUNNER_SHAPE_REJECTED'
    }

    $transformedTokens = $null
    $transformedErrors = $null
    $transformedAst = [Management.Automation.Language.Parser]::ParseInput(
        $TransformedSource,
        [ref]$transformedTokens,
        [ref]$transformedErrors
    )
    if (@($transformedErrors).Count -ne 0) {
        throw 'TASK050_CANONICAL_LATTICED_RUNNER_SHAPE_REJECTED'
    }
    if (
        [regex]::Matches(
            $TransformedSource,
            [regex]::Escape('TASK050_AUTONOMY_ATOMICITY_FAULT_MATRIX_OK boundaries=8')
        ).Count -ne 1 -or
        [regex]::Matches(
            $TransformedSource,
            [regex]::Escape('TASK050_AUTONOMY_ATOMICITY_FAULT_MATRIX_REJECTED')
        ).Count -ne 1
    ) {
        throw 'TASK050_AUTONOMY_ATOMICITY_FAULT_MATRIX_SHAPE_REJECTED'
    }
    $phaseHooks = @($transformedAst.FindAll({
        param($node)
        $node -is [Management.Automation.Language.CommandAst] -and
        $node.GetCommandName() -ceq 'Invoke-Task050CanonicalLatticedPhase'
    }, $true))
    $fixtureHooks = @($transformedAst.FindAll({
        param($node)
        $node -is [Management.Automation.Language.CommandAst] -and
        $node.GetCommandName() -ceq 'Invoke-Task050RuntimeProfileFixture'
    }, $true))
    $matrixHooks = @($transformedAst.FindAll({
        param($node)
        $node -is [Management.Automation.Language.CommandAst] -and
        $node.GetCommandName() -ceq 'Invoke-Task050ProceedWriterMatrix'
    }, $true))
    $initialFixtureHooks = @($fixtureHooks | Where-Object {
        $_.Extent.Text -ceq "Invoke-Task050RuntimeProfileFixture -Cargo `$cargoCommand.Source -RepositoryRoot `$repositoryRoot -Phase 'initial' -EvidenceRoot `$clusterRoot"
    })
    $restartFixtureHooks = @($fixtureHooks | Where-Object {
        $_.Extent.Text -ceq "Invoke-Task050RuntimeProfileFixture -Cargo `$cargoCommand.Source -RepositoryRoot `$repositoryRoot -Phase 'restart' -EvidenceRoot `$clusterRoot"
    })
    $initialHooks = @($phaseHooks | Where-Object {
        $_.Extent.Text -ceq "Invoke-Task050CanonicalLatticedPhase -Phase 'initial' -TestOutput `$task050InitialProfileOutput -EvidenceRoot `$clusterRoot"
    })
    $restartHooks = @($phaseHooks | Where-Object {
        $_.Extent.Text -ceq "Invoke-Task050CanonicalLatticedPhase -Phase 'restart' -TestOutput `$task050RestartProfileOutput -EvidenceRoot `$clusterRoot"
    })
    $restartMatrixHooks = @($matrixHooks | Where-Object {
        $_.Extent.Text -ceq "Invoke-Task050ProceedWriterMatrix -Cargo `$cargoCommand.Source -RepositoryRoot `$repositoryRoot -EvidenceRoot `$clusterRoot -TestOutput `$task050RestartProfileOutput"
    })
    if (
        $fixtureHooks.Count -ne 2 -or
        $initialFixtureHooks.Count -ne 1 -or
        $restartFixtureHooks.Count -ne 1 -or
        $phaseHooks.Count -ne 2 -or
        $initialHooks.Count -ne 1 -or
        $restartHooks.Count -ne 1 -or
        $matrixHooks.Count -ne 1 -or
        $restartMatrixHooks.Count -ne 1
    ) {
        throw 'TASK050_CANONICAL_LATTICED_RUNNER_SHAPE_REJECTED'
    }
    if (
        $restartFixtureHooks[0].Extent.EndOffset -ge $restartMatrixHooks[0].Extent.StartOffset -or
        $restartMatrixHooks[0].Extent.EndOffset -ge $restartHooks[0].Extent.StartOffset
    ) {
        throw 'TASK050_PROCEED_WRITER_MATRIX_ORDER_REJECTED'
    }

    $selfTestOutput = [Collections.Generic.List[string]]::new()
    foreach ($phase in $script:Task050CanonicalPhases) {
        foreach ($profile in $script:Task050AutonomyProfiles) {
            $taskRef = if ($profile -ceq 'ASK_USER') { '1' * 64 } else { '2' * 64 }
            $marker = [ordered]@{
                schema = 'lattice.task050.latticed-profile-input.v2'
                phase = $phase
                profile = $profile
                database_run_id = '05000000000000000000000000000001'
                task_ref = $taskRef
                task_spec_digest = [string]$script:Task050ProfileTaskSpecDigests[$profile]
                autonomy_projection_sha256 = if ($profile -ceq 'ASK_USER') { '3' * 64 } else { '4' * 64 }
                ingress_profile_sha256 = '5' * 64
                daemon_instance_id = 'task050-self-test'
                daemon_epoch = 50
                authority_revision = 50
                observation_digest = '6' * 64
                authority_head_digest = '7' * 64
                expected_status = [ordered]@{
                    schema_version = 'lattice.task.status.v1'
                    status = 'RECONCILIATION_REQUIRED'
                    task_state = 'DRAFT'
                    task_ref = $taskRef
                    ledger_head_digest = '8' * 64
                    result_digest = $null
                }
            }
            $selfTestOutput.Add($script:Task050ProfileMarkerPrefix +
                ($marker | ConvertTo-Json -Compress -Depth 6))
        }
        $parsed = @(Get-Task050PhaseProfileInputs -Phase $phase -TestOutput @($selfTestOutput | Where-Object {
            $_ -cmatch ('"phase":"' + $phase + '"')
        }))
        if ($parsed.Count -ne 2) {
            throw 'TASK050_CANONICAL_LATTICED_RUNNER_SHAPE_REJECTED'
        }
    }
    $restartSelfTestOutput = @($selfTestOutput | Where-Object {
        $_ -cmatch '"phase":"restart"'
    })
    $authorityProfile =
        Get-Task050ProceedWriterMatrixAuthorityProfile -TestOutput $restartSelfTestOutput
    $authorityEnvironment = [ordered]@{
        LATTICE_STORE_DAEMON_INSTANCE_ID = 'task050-self-test'
        LATTICE_STORE_DAEMON_EPOCH = '50'
        LATTICE_STORE_AUTHORITY_REVISION = '50'
        LATTICE_STORE_OBSERVATION_DIGEST = '6' * 64
        LATTICE_STORE_AUTHORITY_HEAD_DIGEST = '7' * 64
    }
    $parentAuthorityEnvironment = [ordered]@{}
    foreach ($name in $authorityEnvironment.Keys) {
        $parentAuthorityEnvironment[$name] =
            [Environment]::GetEnvironmentVariable([string]$name, 'Process')
    }
    $matrixStartInfo = New-Task050ProceedWriterMatrixProcessStartInfo `
        -Cargo 'C:\task050-shape\cargo.exe' `
        -RepositoryRoot $repositoryRoot `
        -AuthorityProfile $authorityProfile
    foreach ($name in $authorityEnvironment.Keys) {
        if (
            [string]$matrixStartInfo.EnvironmentVariables[[string]$name] -cne
                [string]$authorityEnvironment[$name] -or
            [Environment]::GetEnvironmentVariable([string]$name, 'Process') -cne
                $parentAuthorityEnvironment[$name]
        ) {
            throw 'TASK050_PROCEED_WRITER_MATRIX_AUTHORITY_SELF_TEST_REJECTED'
        }
    }
    if (
        $matrixStartInfo.UseShellExecute -or
        -not $matrixStartInfo.RedirectStandardOutput -or
        -not $matrixStartInfo.RedirectStandardError
    ) {
        throw 'TASK050_PROCEED_WRITER_MATRIX_AUTHORITY_SELF_TEST_REJECTED'
    }
    $mutatedRestartSelfTestOutput = @($restartSelfTestOutput)
    $mutatedAuthorityProfile =
        $mutatedRestartSelfTestOutput[1].Substring($script:Task050ProfileMarkerPrefix.Length) |
        ConvertFrom-Json -ErrorAction Stop
    $mutatedAuthorityProfile.authority_head_digest = 'a' * 64
    $mutatedRestartSelfTestOutput[1] = $script:Task050ProfileMarkerPrefix +
        ($mutatedAuthorityProfile | ConvertTo-Json -Compress -Depth 6)
    $mismatchedAuthorityRejected = $false
    try {
        $null = Get-Task050ProceedWriterMatrixAuthorityProfile `
            -TestOutput $mutatedRestartSelfTestOutput
    }
    catch {
        $mismatchedAuthorityRejected = $_.Exception.Message -cin @(
            'TASK050_PROCEED_WRITER_MATRIX_AUTHORITY_REJECTED',
            'TASK050_LATTICED_PROFILE_REPLAY_REJECTED'
        )
    }
    if (-not $mismatchedAuthorityRejected) {
        throw 'TASK050_PROCEED_WRITER_MATRIX_AUTHORITY_SELF_TEST_REJECTED'
    }
    $matrixMarker =
        'TASK050_POSTGRES_PROCEED_WRITER_MATRIX_OK current=1 stale=1 wrong_fence=1 substituted=1 exact_retry=1'
    Assert-Task050ProceedWriterMatrixOutput -ExitCode 0 -Output ($matrixMarker + "`n")
    Assert-Task050ProceedWriterMatrixOutput -ExitCode 0 -Output (
        'test task050_proceed_requires_current_writer_and_retries_without_currentness_when_provisioned ... ' +
        $matrixMarker + "`n"
    )
    foreach ($invalidMatrixOutput in @(
        ('ERROR: ' + $matrixMarker + "`n")
        ($matrixMarker + ' unexpected' + "`n")
        ($matrixMarker + "`n" + $matrixMarker + "`n")
    )) {
        $invalidMatrixOutputRejected = $false
        try { Assert-Task050ProceedWriterMatrixOutput -ExitCode 0 -Output $invalidMatrixOutput }
        catch {
            $invalidMatrixOutputRejected =
                $_.Exception.Message -ceq 'TASK050_PROCEED_WRITER_MATRIX_REJECTED'
        }
        if (-not $invalidMatrixOutputRejected) {
            throw 'TASK050_PROCEED_WRITER_MATRIX_OUTPUT_SELF_TEST_REJECTED'
        }
    }
    $missingMarkerRejected = $false
    try { $null = Get-Task050PhaseProfileInputs -Phase 'initial' -TestOutput @() }
    catch {
        $missingMarkerRejected = $_.Exception.Message -clike 'TASK050_LATTICED_PROFILE_INPUT_REQUIRED*'
    }
    if (-not $missingMarkerRejected) {
        throw 'TASK050_CANONICAL_LATTICED_RUNNER_SHAPE_REJECTED'
    }
    $shapeEvidenceRoot = Join-Path $repositoryRoot ('target\.task050-session-shape-' + [Guid]::NewGuid().ToString('N'))
    [IO.Directory]::CreateDirectory($shapeEvidenceRoot) | Out-Null
    try {
        $shapeEvidence = New-Task050SessionEvidence `
            -EvidenceRoot $shapeEvidenceRoot `
            -Phase 'initial' `
            -Profile 'ASK_USER' `
            -SafeConfigSha256 ('9' * 64)
        if (
            -not (Test-Path -LiteralPath ([string]$shapeEvidence.acceptance_path) -PathType Leaf) -or
            -not (Test-Path -LiteralPath ([string]$shapeEvidence.effect_path) -PathType Leaf)
        ) {
            throw 'TASK050_SESSION_EVIDENCE_SHAPE_REJECTED'
        }
        Remove-Task050SessionEvidence -Evidence $shapeEvidence
    }
    finally {
        if (Test-Path -LiteralPath $shapeEvidenceRoot -PathType Container) {
            [IO.Directory]::Delete($shapeEvidenceRoot, $false)
        }
        if (Test-Path -LiteralPath $shapeEvidenceRoot) {
            throw 'TASK050_SESSION_EVIDENCE_SHAPE_REJECTED'
        }
    }
    $mcpFrames = @((New-Task050McpInput -TaskRef ('9' * 64)) -split "`n" | Where-Object { $_ -cne '' })
    if (
        $mcpFrames.Count -ne 3 -or
        [string](($mcpFrames[0] | ConvertFrom-Json).method) -cne 'server/discover' -or
        [string](($mcpFrames[1] | ConvertFrom-Json).method) -cne 'tools/list' -or
        [string](($mcpFrames[2] | ConvertFrom-Json).params.name) -cne 'lattice_task_status'
    ) {
        throw 'TASK050_CANONICAL_LATTICED_RUNNER_SHAPE_REJECTED'
    }
    Assert-Task050FourToolDiscovery -Tools @($script:Task050ExpectedTools | ForEach-Object {
        [pscustomobject]@{ name = $_ }
    })
    Assert-Task050ObservedEffectEvidence -Evidence ([pscustomobject]@{
        session_counters = [pscustomobject]@{
            dispatch = 1; database = 1; filesystem = 0
            process = 0; network = 1; codex = 0
        }
    })
    $responseFixture = ((@(
        [ordered]@{ jsonrpc = '2.0'; id = 1; result = [ordered]@{} },
        [ordered]@{ jsonrpc = '2.0'; id = 2; result = [ordered]@{} },
        [ordered]@{ jsonrpc = '2.0'; id = 3; result = [ordered]@{} }
    ) | ForEach-Object { $_ | ConvertTo-Json -Compress -Depth 4 }) -join "`n") + "`n"
    $parsedResponses = @(Get-Task050McpResponses -Output $responseFixture)
    if ([long](Get-Task050McpResponse -Responses $parsedResponses -Id 3).id -ne 3) {
        throw 'TASK050_CANONICAL_LATTICED_RUNNER_SHAPE_REJECTED'
    }
    $startupDiagnostics = Get-Task050ExpectedStartupDiagnostics
    Assert-Task050SafeStartupDiagnostics -Diagnostics $startupDiagnostics
    $mutatedStartupDiagnostics = $startupDiagnostics.Replace(
        '"failure_classification":"NONE"',
        '"failure_classification":"LATTICED_CONFIGURATION_REJECTED"'
    )
    $mutatedStartupDiagnosticsRejected = $false
    try { Assert-Task050SafeStartupDiagnostics -Diagnostics $mutatedStartupDiagnostics }
    catch {
        $mutatedStartupDiagnosticsRejected =
            $_.Exception.Message -ceq 'TASK050_LATTICED_STARTUP_DIAGNOSTIC_REJECTED'
    }
    if (-not $mutatedStartupDiagnosticsRejected) {
        throw 'TASK050_LATTICED_STARTUP_DIAGNOSTIC_SELF_TEST_REJECTED'
    }
}

$source = [Text.UTF8Encoding]::new($false, $true).GetString(
    [IO.File]::ReadAllBytes($task019Path)
)

$sourceWithoutCrLf = $source.Replace("`r`n", '')
$hasCrLf = $source.Contains("`r`n")
$hasBareLf = $sourceWithoutCrLf.Contains("`n")
$hasBareCr = $sourceWithoutCrLf.Contains("`r")
if (
    $hasBareCr -or
    ($hasCrLf -and $hasBareLf) -or
    (-not $hasCrLf -and -not $hasBareLf)
) {
    throw 'TASK050_TASK019_HARNESS_NEWLINE_REJECTED'
}
$sourceNewline = if ($hasCrLf) { "`r`n" } else { "`n" }

function Replace-Task050ExactSourceShape {
    param(
        [Parameter(Mandatory = $true)][string]$InputSource,
        [Parameter(Mandatory = $true)][string]$OldShape,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$NewShape,
        [Parameter(Mandatory = $true)][ValidateRange(1, 64)][int]$ExpectedCount,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $actualCount = [regex]::Matches(
        $InputSource,
        [regex]::Escape($OldShape),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count
    if ($actualCount -ne $ExpectedCount) {
        throw $FailureCode
    }
    $replaced = $InputSource.Replace($OldShape, $NewShape)
    $expectedResidualCount = $ExpectedCount * [regex]::Matches(
        $NewShape,
        [regex]::Escape($OldShape),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count
    $actualResidualCount = [regex]::Matches(
        $replaced,
        [regex]::Escape($OldShape),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count
    if ($actualResidualCount -ne $expectedResidualCount) {
        throw $FailureCode
    }
    return $replaced
}

# The managed Windows runner denies Win32_Process CIM inspection even for the
# harness-owned postmaster. Preserve listener, executable hash, native file
# identity, process start time, SQL system identifier, and restart checks while
# using the process API available to this local acceptance boundary.
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape '$process = Get-CimInstance -ClassName Win32_Process -Filter (''ProcessId = '' + $processId) -ErrorAction Stop' `
    -NewShape '$process = Get-Process -Id $processId -ErrorAction Stop' `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_TASK019_PROCESS_SHAPE_REJECTED'
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape '$executable = Get-CanonicalPath -Path ([string]$process.ExecutablePath)' `
    -NewShape '$executable = Get-CanonicalPath -Path ([string]$process.Path)' `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_TASK019_EXECUTABLE_SHAPE_REJECTED'
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape '$createdAt = ([DateTimeOffset]([DateTime]$process.CreationDate)).ToUniversalTime()' `
    -NewShape '$createdAt = ([DateTimeOffset]$process.StartTime).ToUniversalTime()' `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_TASK019_PROCESS_TIME_SHAPE_REJECTED'
if ($source.Contains('$process.CommandLine')) {
    throw 'TASK050_TASK019_COMMAND_LINE_SHAPE_REJECTED'
}

$suiteLoop = '    foreach ($suite in $liveSuites) {'
if (-not $source.Contains($suiteLoop)) {
    throw 'TASK050_TASK019_HARNESS_SHAPE_REJECTED'
}
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape $suiteLoop `
    -NewShape ('    $liveSuites = @([pscustomobject]@{ Name = ''store''; Package = ''lattice-postgres-store''; Test = ''postgres_task_ledger'' })' + $sourceNewline + $suiteLoop) `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_TASK019_HARNESS_SHAPE_REJECTED'
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape "'--test', 'postgres_live'," `
    -NewShape "'--test', [string]`$suite.Test," `
    -ExpectedCount 3 `
    -FailureCode 'TASK050_TASK019_TEST_SHAPE_REJECTED'
$initialLiveTest = '        $initialOutput = Invoke-LiveTest -Cargo $cargoCommand.Source -RepositoryRoot $repositoryRoot -Phase ''initial'''
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape $initialLiveTest `
    -NewShape ($initialLiveTest + $sourceNewline +
        '        if (@($initialOutput | Where-Object { [string]$_ -ceq ''TASK050_AUTONOMY_ATOMICITY_FAULT_MATRIX_OK boundaries=8'' }).Count -ne 1) {' + $sourceNewline +
        '            throw ''TASK050_AUTONOMY_ATOMICITY_FAULT_MATRIX_REJECTED''' + $sourceNewline +
        '        }' + $sourceNewline +
        '        $task050InitialProfileOutput = Invoke-Task050RuntimeProfileFixture -Cargo $cargoCommand.Source -RepositoryRoot $repositoryRoot -Phase ''initial'' -EvidenceRoot $clusterRoot' + $sourceNewline +
        '        Invoke-Task050CanonicalLatticedPhase -Phase ''initial'' -TestOutput $task050InitialProfileOutput -EvidenceRoot $clusterRoot') `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_CANONICAL_LATTICED_INITIAL_HOOK_SHAPE_REJECTED'
$restartLiveTest = '        $null = Invoke-LiveTest -Cargo $cargoCommand.Source -RepositoryRoot $repositoryRoot -Phase ''restart'''
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape $restartLiveTest `
    -NewShape ($restartLiveTest + $sourceNewline +
        '        $task050RestartProfileOutput = Invoke-Task050RuntimeProfileFixture -Cargo $cargoCommand.Source -RepositoryRoot $repositoryRoot -Phase ''restart'' -EvidenceRoot $clusterRoot' + $sourceNewline +
        '        Invoke-Task050ProceedWriterMatrix -Cargo $cargoCommand.Source -RepositoryRoot $repositoryRoot -EvidenceRoot $clusterRoot -TestOutput $task050RestartProfileOutput' + $sourceNewline +
        '        Invoke-Task050CanonicalLatticedPhase -Phase ''restart'' -TestOutput $task050RestartProfileOutput -EvidenceRoot $clusterRoot') `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_CANONICAL_LATTICED_RESTART_HOOK_SHAPE_REJECTED'
$cleanup = '        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword' + $sourceNewline +
    '        $oneTimePassword = $null'
$cleanupReplacement = '        if (-not [string]::IsNullOrEmpty($oneTimePassword)) {' + $sourceNewline +
    '            Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword' + $sourceNewline +
    '        }' + $sourceNewline +
    '        $oneTimePassword = $null'
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape $cleanup `
    -NewShape $cleanupReplacement `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_TASK019_CLEANUP_SHAPE_REJECTED'
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape ('        if (Test-Path -LiteralPath $clusterRoot) {' + $sourceNewline +
        '            $cleanupTargetIsExact = Test-SafeCleanupTarget') `
    -NewShape ('        if ($null -ne $cleanupContainment -and (Test-Path -LiteralPath $clusterRoot)) {' + $sourceNewline +
        '            $cleanupTargetIsExact = Test-SafeCleanupTarget') `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_TASK019_CONTAINMENT_SHAPE_REJECTED'

$scriptRootLiteral = "'" + $PSScriptRoot.Replace("'", "''") + "'"
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape '$PSScriptRoot' `
    -NewShape $scriptRootLiteral `
    -ExpectedCount 13 `
    -FailureCode 'TASK050_TASK019_SCRIPT_ROOT_SHAPE_REJECTED'

if ($SelfTestOnly) {
    [scriptblock]::Create($source) | Out-Null
    Assert-Task050CanonicalLatticedRunnerShape -TransformedSource $source
    Write-Output 'TASK050_TASK019_SOURCE_TRANSFORM_SELF_TEST=PASS'
    Write-Output 'TASK050_CANONICAL_LATTICED_RUNNER_SHAPE_SELF_TEST=PASS'
    Write-Output 'TASK050_CANONICAL_LATTICED_SESSION_SELF_TEST=PASS'
    Write-Output 'TASK050_LATTICED_STARTUP_DIAGNOSTIC_SELF_TEST=PASS'
    Write-Output 'TASK050_PROCEED_WRITER_MATRIX_SHAPE_SELF_TEST=PASS'
    Write-Output 'TASK050_AUTONOMY_ATOMICITY_FAULT_MATRIX_SHAPE_SELF_TEST=PASS'
    return
}

if (-not $script:Task050CanonicalExecutionImplemented) {
    throw 'TASK050_CANONICAL_LATTICED_EXECUTION_NOT_IMPLEMENTED'
}

Push-Location $repositoryRoot
$previousTask050Live = [Environment]::GetEnvironmentVariable('LATTICE_TASK050_LIVE', 'Process')
try {
    [Environment]::SetEnvironmentVariable('LATTICE_TASK050_LIVE', '1', 'Process')
    $harness = [scriptblock]::Create($source)
    $completed = $false
    foreach ($attempt in 1..2) {
        try {
            & $harness -StoreOnly
            if ($LASTEXITCODE -ne 0) {
                throw 'TASK050_POSTGRES_HARNESS_REJECTED'
            }
            if ($script:Task050ObservedProcessIds.Count -ne 4) {
                throw 'TASK050_LATTICED_FOUR_FRESH_PID_REJECTED'
            }
            $completed = $true
            break
        }
        catch {
            if ($attempt -eq 2 -or $_.Exception.Message -cne 'TASK019_HOLDER_ACL_REJECTED') {
                throw
            }
            Start-Sleep -Milliseconds 200
        }
    }
    if (-not $completed) {
        throw 'TASK050_POSTGRES_HARNESS_REJECTED'
    }
}
finally {
    [Environment]::SetEnvironmentVariable('LATTICE_TASK050_LIVE', $previousTask050Live, 'Process')
    Pop-Location
}

Write-Output 'TASK050_POSTGRES_FRESH_PROCESS_ACCEPTANCE=PASS'
