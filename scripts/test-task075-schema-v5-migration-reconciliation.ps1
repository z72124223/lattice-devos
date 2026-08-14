[CmdletBinding()]
param(
    [switch]$SelfTestOnly,
    [switch]$StaticOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Task075LiveInvocationCount = 0
$script:Task075AuthorityEnvironmentNames = @(
    'LATTICE_P0_CONSUMER_SESSION_ID',
    'LATTICE_MEMORY_CATALOG_SIGNATURE_URL',
    'LATTICE_STORE_AUTHORITY_HEAD_DIGEST',
    'LATTICE_STORE_AUTHORITY_REVISION',
    'LATTICE_STORE_CATALOG_SIGNATURE_URL',
    'LATTICE_STORE_DAEMON_EPOCH',
    'LATTICE_STORE_DAEMON_INSTANCE_ID',
    'LATTICE_STORE_OBSERVATION_DIGEST',
    'LATTICE_STORE_PROFILE_EXPECTED',
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
    'LATTICE_TASK068_EXPECTED_RECEIPT_SHA256',
    'LATTICE_TASK075_CURRENT_CATALOG_ONLY',
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
    throw 'TASK075_ACCEPTANCE_MODE_CONFLICT'
}

function Assert-Task075AuthorityEnvironmentVacant {
    foreach ($name in $script:Task075AuthorityEnvironmentNames) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if (-not [string]::IsNullOrEmpty($value)) {
            throw ('TASK075_AMBIENT_AUTHORITY_ENV_REJECTED_' + $name)
        }
    }
}

$script:Task075ReceiptPropertyNames = @(
    'schema',
    'event_type',
    'session_id',
    'consumer_session_id',
    'run_id',
    'host',
    'port',
    'excluded_ports',
    'deadline_utc',
    'nonce_commitment',
    'ordinal',
    'observed_at_utc',
    'payload',
    'payload_sha256',
    'previous_hmac_sha256',
    'event_hmac_sha256'
)
$script:Task075ReceiptEventTypes = @(
    'HOLDER_OPEN',
    'MARKER_CREATED',
    'INITIAL_POSTMASTER_READY',
    'INITIAL_POSTMASTER_STOPPED',
    'RESTART_POSTMASTER_READY',
    'HOLDER_STOP_REQUESTED',
    'HOLDER_STOPPED',
    'CLEANUP_REQUESTED',
    'CLEANUP_COMPLETED',
    'RECEIPT_CLOSED'
)

function Test-Task075ExactSequence {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Actual,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Expected
    )

    if ($Actual.Count -ne $Expected.Count) {
        return $false
    }
    for ($index = 0; $index -lt $Expected.Count; $index++) {
        if ([string]$Actual[$index] -cne [string]$Expected[$index]) {
            return $false
        }
    }
    return $true
}

function Get-Task075GatePlan {
    return @(
        [pscustomobject][ordered]@{
            Name = 'FORMAT'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @('fmt', '--all', '--', '--check')
            FailureCode = 'TASK075_FORMAT_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'STRICT_CLIPPY'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @(
                'clippy', '-p', 'lattice-project-registry', '-p', 'lattice-contracts',
                '-p', 'lattice-postgres-store', '-p', 'lattice-postgres-codebase-memory',
                '--all-targets', '--no-deps', '--locked', '--', '-D', 'warnings'
            )
            FailureCode = 'TASK075_STRICT_CLIPPY_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'PURE_REGISTRY'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @('test', '-p', 'lattice-project-registry', '--locked')
            FailureCode = 'TASK075_PURE_REGISTRY_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'MIGRATION_CONTRACT'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @('test', '-p', 'lattice-postgres-store', '--test', 'migration_contract', '--locked')
            FailureCode = 'TASK075_MIGRATION_CONTRACT_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'REGISTRY_DURABILITY'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @('test', '-p', 'lattice-postgres-store', '--test', 'postgres_project_registry', '--locked')
            FailureCode = 'TASK075_REGISTRY_DURABILITY_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'LEDGER_AUTONOMY_DURABILITY'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @('test', '-p', 'lattice-postgres-store', '--test', 'postgres_task_ledger', '--locked')
            FailureCode = 'TASK075_LEDGER_AUTONOMY_DURABILITY_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'MEMORY_CONTRACTS'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @(
                'test', '-p', 'lattice-contracts', '--test', 'graph_memory_contracts',
                '--test', 'graph_memory_normalized_contracts', '--locked'
            )
            FailureCode = 'TASK075_MEMORY_CONTRACTS_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'MEMORY_EXTENSION_CONTRACT'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @(
                'test', '-p', 'lattice-postgres-codebase-memory', '--test', 'extension_contract',
                '--test', 'setup_api', '--test', 'adapter_api', '--locked'
            )
            FailureCode = 'TASK075_MEMORY_EXTENSION_CONTRACT_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'MEMORY_POSTGRES_LIVE_CONTRACT'
            Type = 'COMMAND'
            Command = 'cargo'
            Arguments = @('test', '-p', 'lattice-postgres-codebase-memory', '--test', 'postgres_live', '--locked')
            FailureCode = 'TASK075_MEMORY_POSTGRES_LIVE_CONTRACT_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'TASK019_SELF_TEST'
            Type = 'TASK019_SELF_TEST'
            Script = 'scripts\run-task019-postgres.ps1'
            Arguments = @('-SelfTestOnly')
            FailureCode = 'TASK075_TASK019_SELF_TEST_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'TASK019_STORE_ONLY'
            Type = 'TASK019_LIVE'
            Profile = 'STORE_ONLY'
            Script = 'scripts\run-task019-postgres.ps1'
            Arguments = @('-StoreOnly')
            FailureCode = 'TASK075_STORE_ONLY_HARNESS_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'TASK019_CATALOG_MEASUREMENT'
            Type = 'TASK019_LIVE'
            Profile = 'CATALOG'
            Script = 'scripts\run-task019-postgres.ps1'
            Arguments = @('-MeasureTask075Catalog')
            FailureCode = 'TASK075_CATALOG_HARNESS_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'TASK019_MEMORY_V3'
            Type = 'TASK019_LIVE'
            Profile = 'MEMORY_V3'
            Script = 'scripts\run-task019-postgres.ps1'
            Arguments = @('-RunTask075MemoryGate')
            FailureCode = 'TASK075_MEMORY_V3_HARNESS_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'TASK050_ACCEPTANCE'
            Type = 'TASK050_LIVE'
            Profile = 'TASK050'
            Script = 'scripts\test-task050-autonomy-receipt-acceptance.ps1'
            Arguments = @()
            FailureCode = 'TASK075_TASK050_ACCEPTANCE_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'REPOSITORY_CHECK'
            Type = 'COMMAND'
            Command = 'npm'
            Arguments = @('run', 'check')
            FailureCode = 'TASK075_REPOSITORY_CHECK_REJECTED'
        },
        [pscustomobject][ordered]@{
            Name = 'DIFF_CHECK'
            Type = 'COMMAND'
            Command = 'git'
            Arguments = @('diff', '--check')
            FailureCode = 'TASK075_DIFF_CHECK_REJECTED'
        }
    )
}

function Resolve-Task075Application {
    param([Parameter(Mandatory = $true)][ValidateSet('cargo', 'npm', 'git', 'powershell')][string]$Name)

    $candidates = switch ($Name) {
        'cargo' { @('cargo.exe', 'cargo') }
        'npm' { @('npm.cmd') }
        'git' { @('git.exe', 'git') }
        'powershell' { @('powershell.exe') }
    }
    foreach ($candidate in $candidates) {
        $command = Get-Command -Name $candidate -CommandType Application -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($null -ne $command) {
            return [string]$command.Source
        }
    }
    throw ('TASK075_REQUIRED_APPLICATION_NOT_FOUND_' + $Name.ToUpperInvariant())
}

function Invoke-Task075StaticGate {
    param([Parameter(Mandatory = $true)]$Gate)

    $executable = Resolve-Task075Application -Name ([string]$Gate.Command)
    Write-Output ('TASK075_STATIC_GATE_ENTER_' + [string]$Gate.Name)
    & $executable @($Gate.Arguments)
    if ($LASTEXITCODE -ne 0) {
        throw [string]$Gate.FailureCode
    }
    Write-Output ('TASK075_STATIC_GATE_PASS_' + [string]$Gate.Name)
}

function Get-Task075SafeDiagnosticTokens {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Output)

    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $tokens = @()
    foreach ($item in $Output) {
        foreach ($match in [regex]::Matches(
            [string]$item,
            '(?<![A-Z0-9_])(?:TASK019|TASK075|STORE|POSTGRES_TASK_LEDGER|POSTGRES_PROJECT_REGISTRY|MEMORY|OPENCLAW)_[A-Z0-9_]{1,63}(?![A-Z0-9_])'
        )) {
            if ($seen.Add($match.Value)) {
                $tokens += $match.Value
            }
        }
    }
    return $tokens
}

function Get-Task075OutputValue {
    param(
        [Parameter(Mandatory = $true)][object[]]$Lines,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$ValuePattern
    )

    $pattern = [regex]::new(('\A' + [regex]::Escape($Name) + '=(?<value>' + $ValuePattern + ')\z'))
    $values = @()
    foreach ($line in $Lines) {
        $match = $pattern.Match([string]$line)
        if ($match.Success) {
            $values += $match.Groups['value'].Value
        }
    }
    if ($values.Count -ne 1) {
        throw ('TASK075_HARNESS_OUTPUT_' + $Name + '_REJECTED')
    }
    return [string]$values[0]
}

function Get-Task075RequiredProperty {
    param(
        [Parameter(Mandatory = $true)]$Value,
        [Parameter(Mandatory = $true)][string]$Name
    )

    $property = $Value.PSObject.Properties[$Name]
    if ($null -eq $property) {
        throw 'TASK075_HOLDER_RECEIPT_SCHEMA_REJECTED'
    }
    return $property.Value
}

function Assert-Task075TrueProperty {
    param(
        [Parameter(Mandatory = $true)]$Value,
        [Parameter(Mandatory = $true)][string]$Name
    )

    $propertyValue = Get-Task075RequiredProperty -Value $Value -Name $Name
    if ($propertyValue -isnot [bool] -or -not [bool]$propertyValue) {
        throw 'TASK075_HOLDER_RECEIPT_CLEANUP_REJECTED'
    }
}

function Get-Task075CatalogSignatures {
    param([Parameter(Mandatory = $true)][object[]]$Output)

    $pattern = [regex]::new(
        '\ATASK075_(?<profile>V5_BARE|V5_MEMORY_V2|V5_MEMORY_V3)_' +
        '(?<source>STORE|MEMORY)_CATALOG_' +
        '(?<metric>RELATION|COLUMN|CONSTRAINT|INDEX|FUNCTION|TABLE_ACL|FUNCTION_ACL|SCHEMA_ACL|AUTONOMY)_' +
        'SIGNATURE=(?<digest>[0-9a-f]{64})\z'
    )
    $expectedGroups = [ordered]@{
        'V5_BARE|STORE' = @('RELATION', 'COLUMN', 'CONSTRAINT', 'INDEX', 'FUNCTION', 'TABLE_ACL', 'FUNCTION_ACL', 'SCHEMA_ACL', 'AUTONOMY')
        'V5_MEMORY_V2|STORE' = @('RELATION', 'COLUMN', 'CONSTRAINT', 'INDEX', 'FUNCTION', 'TABLE_ACL', 'FUNCTION_ACL', 'SCHEMA_ACL', 'AUTONOMY')
        'V5_MEMORY_V2|MEMORY' = @('RELATION', 'COLUMN', 'CONSTRAINT', 'INDEX', 'FUNCTION', 'TABLE_ACL', 'FUNCTION_ACL', 'SCHEMA_ACL')
        'V5_MEMORY_V3|STORE' = @('RELATION', 'COLUMN', 'CONSTRAINT', 'INDEX', 'FUNCTION', 'TABLE_ACL', 'FUNCTION_ACL', 'SCHEMA_ACL', 'AUTONOMY')
        'V5_MEMORY_V3|MEMORY' = @('RELATION', 'COLUMN', 'CONSTRAINT', 'INDEX', 'FUNCTION', 'TABLE_ACL', 'FUNCTION_ACL', 'SCHEMA_ACL')
    }
    $signatures = @($Output | ForEach-Object { [string]$_ } | Where-Object { $pattern.IsMatch($_) })
    if ($signatures.Count -ne 43) {
        throw 'TASK075_CATALOG_SIGNATURE_COUNT_REJECTED'
    }
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($signature in $signatures) {
        $match = $pattern.Match($signature)
        $group = $match.Groups['profile'].Value + '|' + $match.Groups['source'].Value
        $metric = $match.Groups['metric'].Value
        if (
            -not $expectedGroups.Contains($group) -or
            $metric -notin @($expectedGroups[$group]) -or
            -not $seen.Add($group + '|' + $metric)
        ) {
            throw 'TASK075_CATALOG_SIGNATURE_SHAPE_REJECTED'
        }
    }
    foreach ($group in $expectedGroups.Keys) {
        foreach ($metric in @($expectedGroups[$group])) {
            if (-not $seen.Contains($group + '|' + $metric)) {
                throw 'TASK075_CATALOG_SIGNATURE_SHAPE_REJECTED'
            }
        }
    }
    return $signatures
}

function Get-Task075ReceiptEvidence {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('STORE_ONLY', 'CATALOG', 'MEMORY_V3', 'TASK050')][string]$Profile,
        [Parameter(Mandatory = $true)][object[]]$Output,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $lines = @($Output | ForEach-Object { [string]$_ })
    foreach ($requiredLine in @(
        'TASK019_POSTGRES_HARNESS=PASS',
        'POSTGRES_VERSION=17.10',
        'ENDPOINT=127.0.0.1:<dynamic-excludes-5432-64272-55432>',
        'PHASES=initial,restart'
    )) {
        if (@($lines | Where-Object { $_ -ceq $requiredLine }).Count -ne 1) {
            throw ('TASK075_' + $Profile + '_HARNESS_OUTPUT_REJECTED')
        }
    }
    if (
        (@($lines | Where-Object { $_ -cmatch '\ASKIP:' }).Count -ne 0) -or
        (@($lines | Where-Object {
            $_ -cmatch '(?<![A-Z0-9_])(?:LIVE_GATE_FAILED|CATALOG_DIAGNOSTIC_FAILED)(?![A-Z0-9_])'
        }).Count -ne 0)
    ) {
        throw ('TASK075_' + $Profile + '_HARNESS_OUTPUT_REJECTED')
    }

    $reportedPath = Get-Task075OutputValue -Lines $lines -Name 'HOLDER_RECEIPT_PATH' -ValuePattern '.+'
    $reportedRawSha256 = Get-Task075OutputValue -Lines $lines -Name 'HOLDER_RECEIPT_RAW_SHA256' -ValuePattern '[0-9a-f]{64}'
    $reportedFinalHmac = Get-Task075OutputValue -Lines $lines -Name 'HOLDER_RECEIPT_FINAL_HMAC_SHA256' -ValuePattern '[0-9a-f]{64}'
    $reportedEventCountText = Get-Task075OutputValue -Lines $lines -Name 'HOLDER_RECEIPT_EVENT_COUNT' -ValuePattern '[0-9]{1,3}'
    $reportedEventCount = [int]$reportedEventCountText

    $expectedRoot = [IO.Path]::GetFullPath((Join-Path $RepositoryRoot 'target\task019-holder-receipts'))
    $receiptPath = [IO.Path]::GetFullPath($reportedPath)
    $receiptParent = [IO.Path]::GetDirectoryName($receiptPath)
    $runId = [IO.Path]::GetFileNameWithoutExtension($receiptPath)
    if (
        -not [StringComparer]::OrdinalIgnoreCase.Equals($receiptParent, $expectedRoot) -or
        [IO.Path]::GetExtension($receiptPath) -cne '.jsonl' -or
        $runId -cnotmatch '\A[0-9a-f]{32}\z' -or
        -not (Test-Path -LiteralPath $receiptPath -PathType Leaf)
    ) {
        throw 'TASK075_HOLDER_RECEIPT_PATH_REJECTED'
    }
    $receiptRootItem = Get-Item -LiteralPath $receiptParent -Force
    $receiptItem = Get-Item -LiteralPath $receiptPath -Force
    if (
        ($receiptRootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
        ($receiptItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
    ) {
        throw 'TASK075_HOLDER_RECEIPT_PATH_REJECTED'
    }
    $actualRawSha256 = (Get-FileHash -LiteralPath $receiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualRawSha256 -cne $reportedRawSha256) {
        throw 'TASK075_HOLDER_RECEIPT_RAW_SHA256_REJECTED'
    }

    $bytes = [IO.File]::ReadAllBytes($receiptPath)
    if (
        $bytes.Length -lt 2 -or $bytes[$bytes.Length - 1] -ne 10 -or
        ($bytes.Length -ge 3 -and $bytes[0] -eq 239 -and $bytes[1] -eq 187 -and $bytes[2] -eq 191) -or
        $bytes -contains 0 -or $bytes -contains 13
    ) {
        throw 'TASK075_HOLDER_RECEIPT_ENCODING_REJECTED'
    }
    try {
        $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
    }
    catch {
        throw 'TASK075_HOLDER_RECEIPT_ENCODING_REJECTED'
    }
    $expectedEventTypes = if ($Profile -ceq 'CATALOG') {
        @(
            'HOLDER_OPEN', 'MARKER_CREATED', 'INITIAL_POSTMASTER_READY',
            'CATALOG_SIGNATURES_MEASURED', 'INITIAL_POSTMASTER_STOPPED',
            'RESTART_POSTMASTER_READY', 'HOLDER_STOP_REQUESTED', 'HOLDER_STOPPED',
            'CLEANUP_REQUESTED', 'CLEANUP_COMPLETED', 'RECEIPT_CLOSED'
        )
    }
    else {
        @($script:Task075ReceiptEventTypes)
    }
    $jsonLines = @($text.Substring(0, $text.Length - 1) -split "`n")
    if ($jsonLines.Count -ne $reportedEventCount -or $jsonLines.Count -ne $expectedEventTypes.Count) {
        throw 'TASK075_HOLDER_RECEIPT_EVENT_COUNT_REJECTED'
    }
    $records = @()
    foreach ($jsonLine in $jsonLines) {
        if ([string]::IsNullOrWhiteSpace($jsonLine)) {
            throw 'TASK075_HOLDER_RECEIPT_ENCODING_REJECTED'
        }
        try {
            $records += $jsonLine | ConvertFrom-Json
        }
        catch {
            throw 'TASK075_HOLDER_RECEIPT_JSON_REJECTED'
        }
    }

    $eventTypes = @($records | ForEach-Object { [string](Get-Task075RequiredProperty -Value $_ -Name 'event_type') })
    if (-not (Test-Task075ExactSequence -Actual $eventTypes -Expected $expectedEventTypes)) {
        throw 'TASK075_HOLDER_RECEIPT_EVENT_SEQUENCE_REJECTED'
    }

    $first = $records[0]
    $expectedSessionId = [string](Get-Task075RequiredProperty -Value $first -Name 'session_id')
    $expectedConsumerSessionId = [string](Get-Task075RequiredProperty -Value $first -Name 'consumer_session_id')
    $expectedNonceCommitment = [string](Get-Task075RequiredProperty -Value $first -Name 'nonce_commitment')
    $expectedDeadline = [string](Get-Task075RequiredProperty -Value $first -Name 'deadline_utc')
    if (
        $expectedSessionId -cnotmatch '\A[0-9a-f]{32}\z' -or
        $expectedConsumerSessionId -cnotmatch '\A[0-9a-f]{32}\z' -or
        $expectedNonceCommitment -cnotmatch '\A[0-9a-f]{64}\z'
    ) {
        throw 'TASK075_HOLDER_RECEIPT_IDENTITY_REJECTED'
    }
    $previousHmac = '0' * 64
    $previousObservedAt = [DateTimeOffset]::MinValue
    $seenEventHmacs = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    for ($index = 0; $index -lt $records.Count; $index++) {
        $record = $records[$index]
        $propertyNames = @($record.PSObject.Properties | ForEach-Object { [string]$_.Name })
        if (-not (Test-Task075ExactSequence -Actual $propertyNames -Expected $script:Task075ReceiptPropertyNames)) {
            throw 'TASK075_HOLDER_RECEIPT_SCHEMA_REJECTED'
        }
        if (
            [string]$record.schema -cne 'lattice.task019.postgres-holder-authority.v1' -or
            [string]$record.session_id -cne $expectedSessionId -or
            [string]$record.consumer_session_id -cne $expectedConsumerSessionId -or
            [string]$record.run_id -cne $runId -or
            [string]$record.host -cne '127.0.0.1' -or
            [long]$record.port -lt 1 -or [long]$record.port -gt 65535 -or
            [long]$record.port -in @(5432, 64272, 55432) -or
            [string]$record.deadline_utc -cne $expectedDeadline -or
            [string]$record.nonce_commitment -cne $expectedNonceCommitment -or
            [long]$record.ordinal -ne ($index + 1) -or
            [string]$record.previous_hmac_sha256 -cne $previousHmac -or
            [string]$record.payload_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$record.event_hmac_sha256 -cnotmatch '\A[0-9a-f]{64}\z'
        ) {
            throw 'TASK075_HOLDER_RECEIPT_CHAIN_REJECTED'
        }
        $excludedPorts = @($record.excluded_ports | ForEach-Object { [long]$_ })
        if (-not (Test-Task075ExactSequence -Actual $excludedPorts -Expected @(5432, 64272, 55432))) {
            throw 'TASK075_HOLDER_RECEIPT_CHAIN_REJECTED'
        }
        $payloadJson = $record.payload | ConvertTo-Json -Compress -Depth 20
        $payloadBytes = [Text.UTF8Encoding]::new($false).GetBytes($payloadJson)
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            $payloadSha256 = ([BitConverter]::ToString($sha.ComputeHash($payloadBytes))).Replace('-', '').ToLowerInvariant()
        }
        finally {
            $sha.Dispose()
        }
        if ($payloadSha256 -cne [string]$record.payload_sha256) {
            throw 'TASK075_HOLDER_RECEIPT_PAYLOAD_SHA256_REJECTED'
        }
        try {
            $observedAt = [DateTimeOffset]::ParseExact(
                [string]$record.observed_at_utc,
                'o',
                [Globalization.CultureInfo]::InvariantCulture,
                [Globalization.DateTimeStyles]::RoundtripKind
            )
            $deadline = [DateTimeOffset]::ParseExact(
                [string]$record.deadline_utc,
                'o',
                [Globalization.CultureInfo]::InvariantCulture,
                [Globalization.DateTimeStyles]::RoundtripKind
            )
        }
        catch {
            throw 'TASK075_HOLDER_RECEIPT_TIME_REJECTED'
        }
        if (
            $observedAt -lt $previousObservedAt -or
            ([string]$record.event_type -ceq 'RESTART_POSTMASTER_READY' -and $observedAt -gt $deadline)
        ) {
            throw 'TASK075_HOLDER_RECEIPT_TIME_REJECTED'
        }
        if (-not $seenEventHmacs.Add([string]$record.event_hmac_sha256)) {
            throw 'TASK075_HOLDER_RECEIPT_CHAIN_REJECTED'
        }
        $previousObservedAt = $observedAt
        $previousHmac = [string]$record.event_hmac_sha256
    }
    if ($previousHmac -cne $reportedFinalHmac) {
        throw 'TASK075_HOLDER_RECEIPT_FINAL_HMAC_REJECTED'
    }

    $recordsByType = @{}
    foreach ($record in $records) {
        $recordsByType[[string]$record.event_type] = $record
    }
    Assert-Task075TrueProperty -Value $recordsByType['INITIAL_POSTMASTER_STOPPED'].payload -Name 'pg_ctl_status_stopped'
    Assert-Task075TrueProperty -Value $recordsByType['INITIAL_POSTMASTER_STOPPED'].payload -Name 'port_listener_absent'
    Assert-Task075TrueProperty -Value $recordsByType['HOLDER_STOP_REQUESTED'].payload -Name 'harness_completed'
    Assert-Task075TrueProperty -Value $recordsByType['HOLDER_STOPPED'].payload -Name 'pg_ctl_status_stopped'
    Assert-Task075TrueProperty -Value $recordsByType['HOLDER_STOPPED'].payload -Name 'listener_absent'
    Assert-Task075TrueProperty -Value $recordsByType['HOLDER_STOPPED'].payload -Name 'harness_completed'
    Assert-Task075TrueProperty -Value $recordsByType['CLEANUP_REQUESTED'].payload -Name 'cleanup_containment_verified'
    Assert-Task075TrueProperty -Value $recordsByType['CLEANUP_REQUESTED'].payload -Name 'harness_completed'
    Assert-Task075TrueProperty -Value $recordsByType['CLEANUP_COMPLETED'].payload -Name 'cluster_root_absent'
    Assert-Task075TrueProperty -Value $recordsByType['CLEANUP_COMPLETED'].payload -Name 'listener_absent'
    Assert-Task075TrueProperty -Value $recordsByType['CLEANUP_COMPLETED'].payload -Name 'harness_completed'
    Assert-Task075TrueProperty -Value $recordsByType['RECEIPT_CLOSED'].payload -Name 'cleanup_complete'
    if ([long](Get-Task075RequiredProperty -Value $recordsByType['RECEIPT_CLOSED'].payload -Name 'final_event_count_before_close') -ne ($records.Count - 1)) {
        throw 'TASK075_HOLDER_RECEIPT_CLEANUP_REJECTED'
    }
    if ($Profile -ceq 'CATALOG') {
        $outputSignatures = @(Get-Task075CatalogSignatures -Output $lines)
        $receiptSignatures = @(
            Get-Task075RequiredProperty -Value $recordsByType['CATALOG_SIGNATURES_MEASURED'].payload -Name 'signatures' |
                ForEach-Object { [string]$_ }
        )
        if (-not (Test-Task075ExactSequence -Actual $receiptSignatures -Expected $outputSignatures)) {
            throw 'TASK075_CATALOG_RECEIPT_SIGNATURES_REJECTED'
        }
    }

    return [pscustomobject][ordered]@{
        profile = $Profile
        path = $receiptPath
        raw_sha256 = $actualRawSha256
        final_hmac_sha256 = $reportedFinalHmac
        event_count = [long]$records.Count
        run_id = $runId
        session_id = $expectedSessionId
        consumer_session_id = $expectedConsumerSessionId
    }
}

function Resolve-Task075ScriptPath {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][ValidateSet(
            'scripts\run-task019-postgres.ps1',
            'scripts\test-task050-autonomy-receipt-acceptance.ps1'
        )][string]$RelativePath
    )

    $path = [IO.Path]::GetFullPath((Join-Path $RepositoryRoot $RelativePath))
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw 'TASK075_REQUIRED_SCRIPT_NOT_FOUND'
    }
    $item = Get-Item -LiteralPath $path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'TASK075_REQUIRED_SCRIPT_PATH_REJECTED'
    }
    return $path
}

function Invoke-Task075Task019SelfTest {
    param([Parameter(Mandatory = $true)][string]$RepositoryRoot)

    $harnessPath = Resolve-Task075ScriptPath `
        -RepositoryRoot $RepositoryRoot `
        -RelativePath 'scripts\run-task019-postgres.ps1'
    $powershell = Resolve-Task075Application -Name 'powershell'
    $output = @(& $powershell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $harnessPath -SelfTestOnly 2>&1)
    $exitCode = $LASTEXITCODE
    if (
        $exitCode -ne 0 -or
        @($output | Where-Object { [string]$_ -ceq 'TASK019_HARNESS_SELF_TEST=PASS' }).Count -ne 1
    ) {
        $safeTokens = @(Get-Task075SafeDiagnosticTokens -Output $output)
        $safeSummary = if ($safeTokens.Count -eq 0) { 'NO_ALLOWLISTED_TOKEN' } else { $safeTokens -join '|' }
        throw ('TASK075_TASK019_SELF_TEST_REJECTED | ' + $safeSummary)
    }
}

function Invoke-Task075HarnessGate {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('STORE_ONLY', 'CATALOG', 'MEMORY_V3')][string]$Profile,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $script:Task075LiveInvocationCount++
    $harnessPath = Resolve-Task075ScriptPath `
        -RepositoryRoot $RepositoryRoot `
        -RelativePath 'scripts\run-task019-postgres.ps1'
    $profileArgument = switch ($Profile) {
        'STORE_ONLY' { '-StoreOnly' }
        'CATALOG' { '-MeasureTask075Catalog' }
        'MEMORY_V3' { '-RunTask075MemoryGate' }
    }
    $powershell = Resolve-Task075Application -Name 'powershell'
    $output = @(& $powershell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $harnessPath $profileArgument 2>&1)
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        $safeTokens = @(Get-Task075SafeDiagnosticTokens -Output $output)
        $safeSummary = if ($safeTokens.Count -eq 0) { 'NO_ALLOWLISTED_TOKEN' } else { $safeTokens -join '|' }
        throw ('TASK075_' + $Profile + '_HARNESS_REJECTED | ' + $safeSummary)
    }
    $evidence = Get-Task075ReceiptEvidence -Profile $Profile -Output $output -RepositoryRoot $RepositoryRoot
    return $evidence
}

function Invoke-Task075Task050Acceptance {
    param([Parameter(Mandatory = $true)][string]$RepositoryRoot)

    $script:Task075LiveInvocationCount++
    $scriptPath = Resolve-Task075ScriptPath `
        -RepositoryRoot $RepositoryRoot `
        -RelativePath 'scripts\test-task050-autonomy-receipt-acceptance.ps1'
    $powershell = Resolve-Task075Application -Name 'powershell'
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        # Windows PowerShell surfaces a child native process's ordinary stderr
        # progress as NativeCommandError records. Capture those records, then
        # decide solely from the child exit code and exact acceptance sentinel.
        $ErrorActionPreference = 'Continue'
        $output = @(& $powershell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $scriptPath 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if (
        $exitCode -ne 0 -or
        @($output | Where-Object { [string]$_ -ceq 'TASK050_AUTONOMY_RECEIPT_ACCEPTANCE=PASS' }).Count -ne 1
    ) {
        $safeTokens = @(Get-Task075SafeDiagnosticTokens -Output $output)
        $safeSummary = if ($safeTokens.Count -eq 0) { 'NO_ALLOWLISTED_TOKEN' } else { $safeTokens -join '|' }
        throw ('TASK075_TASK050_ACCEPTANCE_REJECTED | ' + $safeSummary)
    }
    return Get-Task075ReceiptEvidence -Profile 'TASK050' -Output $output -RepositoryRoot $RepositoryRoot
}

function Assert-Task075DistinctReceiptEvidence {
    param([Parameter(Mandatory = $true)][object[]]$Evidence)

    if ($Evidence.Count -lt 2) {
        throw 'TASK075_HOLDER_RECEIPT_SET_REJECTED'
    }
    foreach ($property in @('path', 'raw_sha256', 'final_hmac_sha256', 'run_id', 'session_id', 'consumer_session_id')) {
        $values = @($Evidence | ForEach-Object { [string]$_.$property })
        if (@($values | Sort-Object -Unique).Count -ne $values.Count) {
            throw 'TASK075_HOLDER_RECEIPTS_NOT_DISTINCT'
        }
    }
}

function Assert-Task075Throws {
    param(
        [Parameter(Mandatory = $true)][scriptblock]$Action,
        [Parameter(Mandatory = $true)][string]$ExpectedCode
    )

    $actual = $null
    try {
        & $Action
    }
    catch {
        $actual = [string]$_.Exception.Message
    }
    if ($actual -cne $ExpectedCode) {
        throw 'TASK075_ACCEPTANCE_SELF_TEST_EXPECTED_REJECTION_MISSING'
    }
}

function New-Task075SelfTestCatalogSignatures {
    $signatures = @()
    foreach ($group in @(
        @('V5_BARE', 'STORE'),
        @('V5_MEMORY_V2', 'STORE'),
        @('V5_MEMORY_V2', 'MEMORY'),
        @('V5_MEMORY_V3', 'STORE'),
        @('V5_MEMORY_V3', 'MEMORY')
    )) {
        $metrics = @('RELATION', 'COLUMN', 'CONSTRAINT', 'INDEX', 'FUNCTION', 'TABLE_ACL', 'FUNCTION_ACL', 'SCHEMA_ACL')
        if ($group[1] -ceq 'STORE') {
            $metrics += 'AUTONOMY'
        }
        foreach ($metric in $metrics) {
            $signatures += 'TASK075_{0}_{1}_CATALOG_{2}_SIGNATURE={3}' -f `
                $group[0], $group[1], $metric, ('b' * 64)
        }
    }
    return $signatures
}

function New-Task075SelfTestReceipt {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [ValidateSet('STORE_ONLY', 'CATALOG')][string]$Profile = 'STORE_ONLY'
    )

    $receiptRoot = [IO.Path]::GetFullPath((Join-Path $RepositoryRoot 'target\task019-holder-receipts'))
    if (-not (Test-Path -LiteralPath $receiptRoot -PathType Container)) {
        New-Item -ItemType Directory -Path $receiptRoot -Force:$false | Out-Null
    }
    $runId = [Guid]::NewGuid().ToString('N')
    $path = Join-Path $receiptRoot ($runId + '.jsonl')
    $sessionId = [Guid]::NewGuid().ToString('N')
    $consumerSessionId = [Guid]::NewGuid().ToString('N')
    $nonceCommitment = 'a' * 64
    $deadline = [DateTimeOffset]::UtcNow.AddMinutes(5)
    $eventTypes = if ($Profile -ceq 'CATALOG') {
        @(
            'HOLDER_OPEN', 'MARKER_CREATED', 'INITIAL_POSTMASTER_READY',
            'CATALOG_SIGNATURES_MEASURED', 'INITIAL_POSTMASTER_STOPPED',
            'RESTART_POSTMASTER_READY', 'HOLDER_STOP_REQUESTED', 'HOLDER_STOPPED',
            'CLEANUP_REQUESTED', 'CLEANUP_COMPLETED', 'RECEIPT_CLOSED'
        )
    }
    else {
        @($script:Task075ReceiptEventTypes)
    }
    $catalogSignatures = if ($Profile -ceq 'CATALOG') { @(New-Task075SelfTestCatalogSignatures) } else { @() }
    $previousHmac = '0' * 64
    $serialized = [Text.StringBuilder]::new()
    for ($index = 0; $index -lt $eventTypes.Count; $index++) {
        $eventType = $eventTypes[$index]
        $payload = switch ($eventType) {
            'CATALOG_SIGNATURES_MEASURED' { [ordered]@{ signatures = $catalogSignatures } }
            'INITIAL_POSTMASTER_STOPPED' { [ordered]@{ pg_ctl_status_stopped = $true; port_listener_absent = $true } }
            'HOLDER_STOP_REQUESTED' { [ordered]@{ harness_completed = $true } }
            'HOLDER_STOPPED' { [ordered]@{ pg_ctl_status_stopped = $true; listener_absent = $true; harness_completed = $true } }
            'CLEANUP_REQUESTED' { [ordered]@{ cleanup_containment_verified = $true; harness_completed = $true } }
            'CLEANUP_COMPLETED' { [ordered]@{ cluster_root_absent = $true; listener_absent = $true; harness_completed = $true } }
            'RECEIPT_CLOSED' { [ordered]@{ final_event_count_before_close = [long]($eventTypes.Count - 1); cleanup_complete = $true } }
            default { [ordered]@{ self_test = $true; event = $eventType } }
        }
        $payloadJson = $payload | ConvertTo-Json -Compress -Depth 20
        $payloadBytes = [Text.UTF8Encoding]::new($false).GetBytes($payloadJson)
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            $payloadSha256 = ([BitConverter]::ToString($sha.ComputeHash($payloadBytes))).Replace('-', '').ToLowerInvariant()
        }
        finally {
            $sha.Dispose()
        }
        $eventHmac = ('{0:x64}' -f ($index + 1))
        $record = [ordered]@{
            schema = 'lattice.task019.postgres-holder-authority.v1'
            event_type = $eventType
            session_id = $sessionId
            consumer_session_id = $consumerSessionId
            run_id = $runId
            host = '127.0.0.1'
            port = 55433L
            excluded_ports = @(5432, 64272, 55432)
            deadline_utc = $deadline.ToString('o')
            nonce_commitment = $nonceCommitment
            ordinal = [long]($index + 1)
            observed_at_utc = $deadline.AddMinutes(-4).AddMilliseconds($index).ToString('o')
            payload = $payload
            payload_sha256 = $payloadSha256
            previous_hmac_sha256 = $previousHmac
            event_hmac_sha256 = $eventHmac
        }
        [void]$serialized.Append(($record | ConvertTo-Json -Compress -Depth 24))
        [void]$serialized.Append("`n")
        $previousHmac = $eventHmac
    }
    [IO.File]::WriteAllBytes($path, [Text.UTF8Encoding]::new($false).GetBytes($serialized.ToString()))
    $outputLines = @(
        'TASK019_POSTGRES_HARNESS=PASS',
        'POSTGRES_VERSION=17.10',
        'ENDPOINT=127.0.0.1:<dynamic-excludes-5432-64272-55432>',
        'PHASES=initial,restart'
    )
    $outputLines += $catalogSignatures
    $outputLines += @(
        ('HOLDER_RECEIPT_PATH=' + $path),
        ('HOLDER_RECEIPT_RAW_SHA256=' + (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()),
        ('HOLDER_RECEIPT_FINAL_HMAC_SHA256=' + $previousHmac),
        ('HOLDER_RECEIPT_EVENT_COUNT=' + [string]$eventTypes.Count)
    )
    return [pscustomobject][ordered]@{
        path = $path
        output = $outputLines
    }
}

function Invoke-Task075AcceptanceSelfTest {
    param([Parameter(Mandatory = $true)][string]$RepositoryRoot)

    $plan = @(Get-Task075GatePlan)
    $expectedNames = @(
        'FORMAT', 'STRICT_CLIPPY', 'PURE_REGISTRY', 'MIGRATION_CONTRACT',
        'REGISTRY_DURABILITY', 'LEDGER_AUTONOMY_DURABILITY', 'MEMORY_CONTRACTS',
        'MEMORY_EXTENSION_CONTRACT', 'MEMORY_POSTGRES_LIVE_CONTRACT',
        'TASK019_SELF_TEST', 'TASK019_STORE_ONLY', 'TASK019_CATALOG_MEASUREMENT',
        'TASK019_MEMORY_V3', 'TASK050_ACCEPTANCE', 'REPOSITORY_CHECK', 'DIFF_CHECK'
    )
    if (
        $plan.Count -ne 16 -or
        -not (Test-Task075ExactSequence -Actual @($plan.Name) -Expected $expectedNames) -or
        @($plan.Name | Sort-Object -Unique).Count -ne $plan.Count -or
        @($plan | Where-Object { $_.Type -ceq 'COMMAND' -and $_.Command -notin @('cargo', 'npm', 'git') }).Count -ne 0
    ) {
        throw 'TASK075_ACCEPTANCE_SELF_TEST_GATE_PLAN_REJECTED'
    }
    $migrationArguments = @($plan | Where-Object { $_.Name -ceq 'MIGRATION_CONTRACT' } | ForEach-Object { $_.Arguments })
    if (-not (Test-Task075ExactSequence -Actual $migrationArguments -Expected @(
        'test', '-p', 'lattice-postgres-store', '--test', 'migration_contract', '--locked'
    ))) {
        throw 'TASK075_ACCEPTANCE_SELF_TEST_GATE_PLAN_REJECTED'
    }
    $memoryPostgresArguments = @($plan | Where-Object { $_.Name -ceq 'MEMORY_POSTGRES_LIVE_CONTRACT' } | ForEach-Object { $_.Arguments })
    if (-not (Test-Task075ExactSequence -Actual $memoryPostgresArguments -Expected @(
        'test', '-p', 'lattice-postgres-codebase-memory', '--test', 'postgres_live', '--locked'
    ))) {
        throw 'TASK075_ACCEPTANCE_SELF_TEST_GATE_PLAN_REJECTED'
    }
    $scriptGates = @($plan | Where-Object { $_.Type -cne 'COMMAND' })
    $expectedScriptTypes = @('TASK019_SELF_TEST', 'TASK019_LIVE', 'TASK019_LIVE', 'TASK019_LIVE', 'TASK050_LIVE')
    $expectedScriptPaths = @(
        'scripts\run-task019-postgres.ps1',
        'scripts\run-task019-postgres.ps1',
        'scripts\run-task019-postgres.ps1',
        'scripts\run-task019-postgres.ps1',
        'scripts\test-task050-autonomy-receipt-acceptance.ps1'
    )
    $expectedScriptArguments = @(
        '-SelfTestOnly', '-StoreOnly', '-MeasureTask075Catalog',
        '-RunTask075MemoryGate', ''
    )
    if (
        -not (Test-Task075ExactSequence -Actual @($scriptGates.Type) -Expected $expectedScriptTypes) -or
        -not (Test-Task075ExactSequence -Actual @($scriptGates.Script) -Expected $expectedScriptPaths) -or
        -not (Test-Task075ExactSequence -Actual @(
            $scriptGates | ForEach-Object { @($_.Arguments) -join "`0" }
        ) -Expected $expectedScriptArguments) -or
        $script:Task075LiveInvocationCount -ne 0
    ) {
        throw 'TASK075_ACCEPTANCE_SELF_TEST_SCRIPT_PLAN_REJECTED'
    }
    $task050Invoker = (Get-Command Invoke-Task075Task050Acceptance -CommandType Function).ScriptBlock.ToString()
    foreach ($requiredShape in @(
        '$previousErrorActionPreference = $ErrorActionPreference',
        '$ErrorActionPreference = ''Continue''',
        '$exitCode = $LASTEXITCODE',
        '$ErrorActionPreference = $previousErrorActionPreference',
        'TASK050_AUTONOMY_RECEIPT_ACCEPTANCE=PASS'
    )) {
        if (-not $task050Invoker.Contains($requiredShape)) {
            throw 'TASK075_ACCEPTANCE_SELF_TEST_TASK050_NATIVE_CAPTURE_REJECTED'
        }
    }

    $fixture = New-Task075SelfTestReceipt -RepositoryRoot $RepositoryRoot
    $catalogFixture = New-Task075SelfTestReceipt -RepositoryRoot $RepositoryRoot -Profile 'CATALOG'
    if (
        $null -eq $fixture -or $null -eq $fixture.PSObject.Properties['output'] -or
        $null -eq $fixture.output -or $null -eq $catalogFixture -or
        $null -eq $catalogFixture.PSObject.Properties['output'] -or $null -eq $catalogFixture.output
    ) {
        throw 'TASK075_ACCEPTANCE_SELF_TEST_FIXTURE_REJECTED'
    }
    try {
        $evidence = Get-Task075ReceiptEvidence -Profile 'STORE_ONLY' -Output $fixture.output -RepositoryRoot $RepositoryRoot
        if ($evidence.event_count -ne 10 -or [string]$evidence.path -cne [string]$fixture.path) {
            throw 'TASK075_ACCEPTANCE_SELF_TEST_RECEIPT_REJECTED'
        }
        $duplicateOutput = @($fixture.output) + @('TASK019_POSTGRES_HARNESS=PASS')
        Assert-Task075Throws -ExpectedCode 'TASK075_STORE_ONLY_HARNESS_OUTPUT_REJECTED' -Action {
            Get-Task075ReceiptEvidence -Profile 'STORE_ONLY' -Output $duplicateOutput -RepositoryRoot $RepositoryRoot
        }
        $badHashOutput = @($fixture.output | ForEach-Object {
            if ($_ -cmatch '\AHOLDER_RECEIPT_RAW_SHA256=') { 'HOLDER_RECEIPT_RAW_SHA256=' + ('f' * 64) } else { $_ }
        })
        Assert-Task075Throws -ExpectedCode 'TASK075_HOLDER_RECEIPT_RAW_SHA256_REJECTED' -Action {
            Get-Task075ReceiptEvidence -Profile 'STORE_ONLY' -Output $badHashOutput -RepositoryRoot $RepositoryRoot
        }
        $catalogEvidence = Get-Task075ReceiptEvidence `
            -Profile 'CATALOG' `
            -Output $catalogFixture.output `
            -RepositoryRoot $RepositoryRoot
        if (
            $catalogEvidence.event_count -ne 11 -or
            [string]$catalogEvidence.path -cne [string]$catalogFixture.path
        ) {
            throw 'TASK075_ACCEPTANCE_SELF_TEST_CATALOG_RECEIPT_REJECTED'
        }
        $catalogSignatures = @(Get-Task075CatalogSignatures -Output $catalogFixture.output)
        Assert-Task075Throws -ExpectedCode 'TASK075_CATALOG_SIGNATURE_COUNT_REJECTED' -Action {
            Get-Task075CatalogSignatures -Output @($catalogSignatures[0..41])
        }
        $diagnosticSecret = 'password-must-not-survive'
        $tokens = @(Get-Task075SafeDiagnosticTokens -Output @(
            'TASK075_STAGE_ENTER_GLOBAL_V5_PENDING',
            ('panic STORE_MIGRATION_HISTORY_MISMATCH ' + $diagnosticSecret)
        ))
        if (
            -not (Test-Task075ExactSequence -Actual $tokens -Expected @(
                'TASK075_STAGE_ENTER_GLOBAL_V5_PENDING', 'STORE_MIGRATION_HISTORY_MISMATCH'
            )) -or
            ($tokens -join '|') -cmatch [regex]::Escape($diagnosticSecret)
        ) {
            throw 'TASK075_ACCEPTANCE_SELF_TEST_DIAGNOSTIC_REDACTION_REJECTED'
        }
    }
    finally {
        if (Test-Path -LiteralPath $fixture.path -PathType Leaf) {
            Remove-Item -LiteralPath $fixture.path -Force
        }
        if (Test-Path -LiteralPath $catalogFixture.path -PathType Leaf) {
            Remove-Item -LiteralPath $catalogFixture.path -Force
        }
    }
    if ($script:Task075LiveInvocationCount -ne 0) {
        throw 'TASK075_ACCEPTANCE_SELF_TEST_STARTED_LIVE_GATE'
    }

    foreach ($ambientName in @('LATTICE_TASK019_LIVE', 'LATTICE_WRITER_LEASE_RUNTIME_URL')) {
        $previousAmbient = [Environment]::GetEnvironmentVariable($ambientName, 'Process')
        try {
            [Environment]::SetEnvironmentVariable($ambientName, 'task075-self-test-sentinel', 'Process')
            Assert-Task075Throws -ExpectedCode ('TASK075_AMBIENT_AUTHORITY_ENV_REJECTED_' + $ambientName) -Action {
                Assert-Task075AuthorityEnvironmentVacant
            }
        }
        finally {
            [Environment]::SetEnvironmentVariable($ambientName, $previousAmbient, 'Process')
        }
    }
    if ($script:Task075LiveInvocationCount -ne 0) {
        throw 'TASK075_ACCEPTANCE_SELF_TEST_STARTED_LIVE_GATE'
    }
}

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot 'Cargo.toml') -PathType Leaf)) {
    throw 'TASK075_REPOSITORY_ROOT_REJECTED'
}

if ($SelfTestOnly) {
    Invoke-Task075AcceptanceSelfTest -RepositoryRoot $repositoryRoot
    Write-Output 'TASK075_SCHEMA_V5_ACCEPTANCE_SELF_TEST=PASS'
    return
}

Push-Location $repositoryRoot
try {
    Assert-Task075AuthorityEnvironmentVacant
    $receiptEvidence = [Collections.Generic.List[object]]::new()
    $storeEvidence = $null
    $catalogEvidence = $null
    $memoryEvidence = $null
    $task050Evidence = $null
    foreach ($gate in @(Get-Task075GatePlan)) {
        switch ([string]$gate.Type) {
            'COMMAND' {
                Invoke-Task075StaticGate -Gate $gate
            }
            'TASK019_SELF_TEST' {
                Write-Output 'TASK075_STATIC_GATE_ENTER_TASK019_SELF_TEST'
                Invoke-Task075Task019SelfTest -RepositoryRoot $repositoryRoot
                Write-Output 'TASK075_STATIC_GATE_PASS_TASK019_SELF_TEST'
            }
            'TASK019_LIVE' {
                if ($StaticOnly) {
                    continue
                }
                Write-Output ('TASK075_LIVE_GATE_ENTER_' + [string]$gate.Profile)
                $evidence = Invoke-Task075HarnessGate `
                    -Profile ([string]$gate.Profile) `
                    -RepositoryRoot $repositoryRoot
                $receiptEvidence.Add($evidence)
                switch ([string]$gate.Profile) {
                    'STORE_ONLY' { $storeEvidence = $evidence }
                    'CATALOG' { $catalogEvidence = $evidence }
                    'MEMORY_V3' { $memoryEvidence = $evidence }
                }
                Write-Output ('TASK075_LIVE_GATE_PASS_' + [string]$gate.Profile)
            }
            'TASK050_LIVE' {
                if ($StaticOnly) {
                    continue
                }
                Write-Output 'TASK075_LIVE_GATE_ENTER_TASK050'
                $task050Evidence = Invoke-Task075Task050Acceptance -RepositoryRoot $repositoryRoot
                $receiptEvidence.Add($task050Evidence)
                Write-Output 'TASK075_LIVE_GATE_PASS_TASK050'
            }
            default {
                throw 'TASK075_GATE_PLAN_TYPE_REJECTED'
            }
        }
    }

    if ($StaticOnly) {
        if ($script:Task075LiveInvocationCount -ne 0) {
            throw 'TASK075_STATIC_MODE_STARTED_LIVE_GATE'
        }
        Write-Output 'TASK075_SCHEMA_V5_STATIC_GATES=PASS'
        return
    }
    if (
        $receiptEvidence.Count -ne 4 -or
        $null -eq $storeEvidence -or $null -eq $catalogEvidence -or
        $null -eq $memoryEvidence -or $null -eq $task050Evidence
    ) {
        throw 'TASK075_ACCEPTANCE_EVIDENCE_SET_REJECTED'
    }
    Assert-Task075DistinctReceiptEvidence -Evidence @($receiptEvidence)

    Write-Output ('TASK075_STORE_ONLY_RECEIPT_PATH=' + [string]$storeEvidence.path)
    Write-Output ('TASK075_STORE_ONLY_RECEIPT_RAW_SHA256=' + [string]$storeEvidence.raw_sha256)
    Write-Output ('TASK075_STORE_ONLY_RECEIPT_FINAL_HMAC_SHA256=' + [string]$storeEvidence.final_hmac_sha256)
    Write-Output ('TASK075_STORE_ONLY_RECEIPT_EVENT_COUNT=' + [string]$storeEvidence.event_count)
    Write-Output ('TASK075_CATALOG_RECEIPT_PATH=' + [string]$catalogEvidence.path)
    Write-Output ('TASK075_CATALOG_RECEIPT_RAW_SHA256=' + [string]$catalogEvidence.raw_sha256)
    Write-Output ('TASK075_CATALOG_RECEIPT_FINAL_HMAC_SHA256=' + [string]$catalogEvidence.final_hmac_sha256)
    Write-Output ('TASK075_CATALOG_RECEIPT_EVENT_COUNT=' + [string]$catalogEvidence.event_count)
    Write-Output ('TASK075_MEMORY_V3_RECEIPT_PATH=' + [string]$memoryEvidence.path)
    Write-Output ('TASK075_MEMORY_V3_RECEIPT_RAW_SHA256=' + [string]$memoryEvidence.raw_sha256)
    Write-Output ('TASK075_MEMORY_V3_RECEIPT_FINAL_HMAC_SHA256=' + [string]$memoryEvidence.final_hmac_sha256)
    Write-Output ('TASK075_MEMORY_V3_RECEIPT_EVENT_COUNT=' + [string]$memoryEvidence.event_count)
    Write-Output ('TASK075_TASK050_RECEIPT_PATH=' + [string]$task050Evidence.path)
    Write-Output ('TASK075_TASK050_RECEIPT_RAW_SHA256=' + [string]$task050Evidence.raw_sha256)
    Write-Output ('TASK075_TASK050_RECEIPT_FINAL_HMAC_SHA256=' + [string]$task050Evidence.final_hmac_sha256)
    Write-Output ('TASK075_TASK050_RECEIPT_EVENT_COUNT=' + [string]$task050Evidence.event_count)
    Write-Output 'TASK075_SCHEMA_V5_MIGRATION_RECONCILIATION_ACCEPTANCE=PASS'
}
finally {
    Pop-Location
}
